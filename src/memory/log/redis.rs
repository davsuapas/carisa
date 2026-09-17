//! Redis implementation of active message and metadata storage.
//!
//! # Design overview
//!
//! Session history is stored in **per-session partitions**. Each partition is
//! a Redis hash whose fields are message ids (ULIDs) and whose values are
//! zstd-compressed JSON payloads. The hash acts as both index and data, so a
//! read needs no separate index lookup.
//!
//! A condensation message closes the current partition and opens a new one
//! whose first entry is that condensation. This keeps the active context
//! small: `history` only ever reads the current partition.
//!
//! # Usage
//!
//! See [`RedisLogStorage::builder`] for a complete construction example with
//! custom compression settings, and [`RedisLogStorage::new`] for the default
//! setup. The examples live on those items so they stay next to the API they
//! describe.
//!
//! # Key layout
//!
//! ```text
//! session:{agent_id}:{session_id}:part:{partition}   HASH<MessageId, payload>
//! session:{agent_id}:{session_id}:current_partition  integer
//! meta:{agent_id}:{session_id}                       JSON SessionMeta
//! ```
//!
//! Partitions are numbered from `0` (`INITIAL_PARTITION`).
//!
//! # Write path (`append`)
//!
//! `append` reads the partition pointer (defaulting to `0`) and writes every
//! message with `HSET` into the active partition, inside one atomic pipeline.
//! A condensation increments the pointer with `INCR` first, so the message
//! lands in the freshly opened partition. If the pointer does not exist yet,
//! it is initialized to `0` in the same pipeline.
//!
//! `append` is expected to receive few messages per call, so messages are
//! compressed one by one.
//!
//! # Read path (`history`)
//!
//! `history` resolves the active partition with `GET` and then reads the whole
//! partition with a single `HGETALL`. Since `HGETALL` does not guarantee field
//! order, decoded messages are sorted by their ULID id, whose lexicographic
//! order matches creation order.
//!
//! # Compression and the blocking pool
//!
//! Payloads are compressed with zstd (default level `3`). Compression and
//! decompression are CPU-bound, so work above `compress_blocking_threshold` is
//! moved to the blocking pool with `tokio::task::spawn_blocking`. The
//! threshold is applied per message when compressing and to the whole batch
//! when decompressing a partition.
//!
//! # Testability
//!
//! The storage is generic over the connection type `C` so tests can inject
//! `redis_test::MockRedisConnection`; production uses
//! `redis::aio::ConnectionManager`.

use std::collections::HashMap;

use async_trait::async_trait;
use redis::{AsyncCommands, Client, aio::ConnectionManager};

use crate::log::{
  LogError, LogMessage, LogMessageKind, MessageId, SessionMeta,
  StoredLogMessage,
  storage::{LogStorage, SessionMetaStore},
};

const SESSION_KEY_PREFIX: &str = "session";
const META_KEY_PREFIX: &str = "meta";
const PART_KEY_SUFFIX: &str = "part";
const CURRENT_PARTITION_KEY_SUFFIX: &str = "current_partition";

/// Number of the first partition created for a session.
const INITIAL_PARTITION: i64 = 0;

const DEFAULT_ZSTD_LEVEL: i32 = 3;
const DEFAULT_COMPRESS_BLOCKING_THRESHOLD: usize = 8 * 1024;

/// Redis storage for session messages and metadata.
///
/// Generic over the connection type `C` so tests can substitute
/// `redis_test::MockRedisConnection` for the real
/// `redis::aio::ConnectionManager`. Production code is unaffected:
/// `RedisLogStorage::new(client)` still returns
/// `RedisLogStorage<ConnectionManager>` (i.e. plain `RedisLogStorage`,
/// via the default type parameter).
#[derive(Debug, Clone)]
pub struct RedisLogStorage<C = ConnectionManager> {
  connection: C,
  zstd_level: i32,
  compress_blocking_threshold: usize,
}

impl RedisLogStorage<ConnectionManager> {
  /// Creates storage from a Redis client, using default settings.
  ///
  /// # Errors
  ///
  /// Returns an error if Redis connection management cannot be initialized.
  ///
  /// # Examples
  ///
  /// ```no_run
  /// use carisa_core::log::redis::RedisLogStorage;
  /// use redis::Client;
  ///
  /// #[tokio::main]
  /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
  ///   let client = Client::open("redis://127.0.0.1:6379")?;
  ///   let storage = RedisLogStorage::new(client).await?;
  ///   Ok(())
  /// }
  /// ```
  pub async fn new(client: Client) -> Result<Self, LogError> {
    Self::builder(client).build().await
  }

  /// Starts a builder for storage with non-default settings.
  ///
  /// # Examples
  ///
  /// ```no_run
  /// use carisa_core::log::redis::RedisLogStorage;
  /// use redis::Client;
  ///
  /// #[tokio::main]
  /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
  ///   let client = Client::open("redis://127.0.0.1:6379")?;
  ///   let storage = RedisLogStorage::builder(client)
  ///     .zstd_level(6)
  ///     .compress_blocking_threshold(64 * 1024)
  ///     .build()
  ///     .await?;
  ///   Ok(())
  /// }
  /// ```
  pub const fn builder(client: Client) -> RedisLogStorageBuilder {
    RedisLogStorageBuilder::new(client)
  }
}

impl<C> RedisLogStorage<C> {
  /// Builds storage directly from an already-constructed connection,
  /// bypassing `Client`/`ConnectionManager`. Used by tests to inject
  /// `MockRedisConnection`; not exposed outside this crate because
  /// production code should go through `new`/`builder`.
  #[cfg(test)]
  pub(crate) const fn from_parts(
    connection: C,
    zstd_level: i32,
    compress_blocking_threshold: usize,
  ) -> Self {
    Self {
      connection,
      zstd_level,
      compress_blocking_threshold,
    }
  }
}

impl<C> RedisLogStorage<C>
where
  C: redis::aio::ConnectionLike + Clone + Send + Sync + 'static,
{
  /// Compresses a single message.
  ///
  /// `append` is expected to receive few messages per call, so compression
  /// stays per message: large payloads are offloaded to the blocking pool
  /// individually, small ones run inline.
  async fn encode_message_sized(
    &self,
    message: &LogMessage,
  ) -> Result<Vec<u8>, LogError> {
    let json = serde_json::to_vec(message)?;
    let level = self.zstd_level;

    if json.len() < self.compress_blocking_threshold {
      return compress(&json, level);
    }

    tokio::task::spawn_blocking(move || compress(&json, level))
      .await
      .map_err(LogError::Task)?
  }
}

/// Builder for configuring a Redis log storage.
///
/// Create one with [`RedisLogStorage::builder`].
#[derive(Debug)]
pub struct RedisLogStorageBuilder {
  client: Client,
  zstd_level: i32,
  compress_blocking_threshold: usize,
}

impl RedisLogStorageBuilder {
  const fn new(client: Client) -> Self {
    Self {
      client,
      zstd_level: DEFAULT_ZSTD_LEVEL,
      compress_blocking_threshold: DEFAULT_COMPRESS_BLOCKING_THRESHOLD,
    }
  }

  /// Sets the Zstandard compression level.
  #[must_use]
  pub const fn zstd_level(mut self, level: i32) -> Self {
    self.zstd_level = level;
    self
  }

  /// Sets the payload size at which compression runs on the blocking pool.
  #[must_use]
  pub const fn compress_blocking_threshold(mut self, threshold: usize) -> Self {
    self.compress_blocking_threshold = threshold;
    self
  }

  /// Builds a configured Redis storage using a managed connection.
  ///
  /// # Errors
  ///
  /// Returns an error if the Redis client cannot create a connection manager.
  pub async fn build(
    self,
  ) -> Result<RedisLogStorage<ConnectionManager>, LogError> {
    Ok(RedisLogStorage {
      connection: self.client.get_connection_manager().await?,
      zstd_level: self.zstd_level,
      compress_blocking_threshold: self.compress_blocking_threshold,
    })
  }
}

#[async_trait]
impl<C> LogStorage for RedisLogStorage<C>
where
  C: redis::aio::ConnectionLike + Clone + Send + Sync + 'static,
{
  async fn append(
    &self,
    agent_id: &str,
    session_id: &str,
    messages: Vec<LogMessage>,
  ) -> Result<Vec<MessageId>, LogError> {
    let ids = messages.iter().map(|message| message.id.clone()).collect();
    if messages.is_empty() {
      return Ok(ids);
    }

    let mut encoded: Vec<(String, Vec<u8>, bool)> =
      Vec::with_capacity(messages.len());
    for message in &messages {
      let id = message.id.clone();
      let is_condensation =
        matches!(&message.kind, LogMessageKind::Condensation { .. });
      let payload = self.encode_message_sized(message).await?;
      encoded.push((id, payload, is_condensation));
    }

    let mut connection = self.connection.clone();
    let pointer_key = current_partition_key(agent_id, session_id);

    let current: Option<i64> = redis::cmd("GET")
      .arg(&pointer_key)
      .query_async(&mut connection)
      .await?;
    let needs_pointer_init = current.is_none();
    let mut partition = current.unwrap_or(INITIAL_PARTITION);

    let mut pipeline = redis::pipe();
    pipeline.atomic();
    if needs_pointer_init {
      pipeline.set(&pointer_key, INITIAL_PARTITION);
    }
    for (id, payload, is_condensation) in &encoded {
      if *is_condensation {
        pipeline.incr(&pointer_key, 1);
        partition = partition.saturating_add(1);
      }
      let part_key = partition_key(agent_id, session_id, partition);
      pipeline.hset(&part_key, id, payload);
    }
    pipeline.query_async::<()>(&mut connection).await?;

    Ok(ids)
  }

  async fn history(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<Vec<LogMessage>, LogError> {
    let mut connection = self.connection.clone();
    let pointer_key = current_partition_key(agent_id, session_id);

    let current: Option<i64> = redis::cmd("GET")
      .arg(&pointer_key)
      .query_async(&mut connection)
      .await?;
    let part_key =
      partition_key(agent_id, session_id, current.unwrap_or(INITIAL_PARTITION));

    let entries: HashMap<String, Vec<u8>> = redis::cmd("HGETALL")
      .arg(&part_key)
      .query_async(&mut connection)
      .await?;

    let payloads: Vec<Vec<u8>> = entries.into_values().collect();
    let total_bytes: usize = payloads.iter().map(Vec::len).sum();

    let mut messages = if total_bytes < self.compress_blocking_threshold {
      decompress_all(payloads)?
    } else {
      tokio::task::spawn_blocking(move || decompress_all(payloads))
        .await
        .map_err(LogError::Task)??
    };

    // `HGETALL` does not guarantee a field order, so restore the
    // chronological order explicitly. Message ids are ULIDs, whose
    // lexicographic order matches their creation order.
    messages.sort_by_key(|message| message.id.clone());

    Ok(messages)
  }

  async fn drain_next_batch(
    &self,
    agent_id: &str,
    session_id: &str,
    batch_index: usize,
  ) -> Result<Option<(usize, Vec<StoredLogMessage>)>, LogError> {
    let mut connection = self.connection.clone();
    let pointer_key = current_partition_key(agent_id, session_id);

    let current: Option<i64> = redis::cmd("GET")
      .arg(&pointer_key)
      .query_async(&mut connection)
      .await?;
    let last_partition = current.unwrap_or(INITIAL_PARTITION);

    let requested_partition = i64::try_from(batch_index)
      .map_err(|_| LogError::InvalidBatchIndex { batch_index })?;
    if requested_partition > last_partition {
      return Ok(None);
    }

    let part_key = partition_key(agent_id, session_id, requested_partition);
    let entries: HashMap<String, Vec<u8>> = redis::cmd("HGETALL")
      .arg(&part_key)
      .query_async(&mut connection)
      .await?;

    if entries.is_empty() {
      return Ok(None);
    }

    let mut messages: Vec<StoredLogMessage> = entries
      .into_iter()
      .map(|(id, payload)| StoredLogMessage { id, payload })
      .collect();
    messages.sort_by_key(|message| message.id.clone());
    Ok(Some((batch_index.saturating_add(1), messages)))
  }

  async fn delete_session(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<(), LogError> {
    let mut connection = self.connection.clone();
    let pointer_key = current_partition_key(agent_id, session_id);

    let current: Option<i64> = redis::cmd("GET")
      .arg(&pointer_key)
      .query_async(&mut connection)
      .await?;

    let mut pipeline = redis::pipe();
    pipeline.atomic();
    pipeline.del(&pointer_key);
    for partition in 0..=current.unwrap_or(INITIAL_PARTITION) {
      pipeline.del(partition_key(agent_id, session_id, partition));
    }
    pipeline.query_async::<()>(&mut connection).await?;

    Ok(())
  }
}

#[async_trait]
impl<C> SessionMetaStore for RedisLogStorage<C>
where
  C: redis::aio::ConnectionLike + Clone + Send + Sync + 'static,
{
  async fn put_meta(&self, meta: &SessionMeta) -> Result<(), LogError> {
    let mut connection = self.connection.clone();
    let serialized = serde_json::to_string(meta)?;

    let _: () = connection
      .set(meta_key(&meta.agent_id, &meta.session_id), serialized)
      .await?;

    Ok(())
  }

  async fn get_meta(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<Option<SessionMeta>, LogError> {
    let mut connection = self.connection.clone();
    let value: Option<String> =
      connection.get(meta_key(agent_id, session_id)).await?;
    value
      .map(|serialized| {
        serde_json::from_str(&serialized).map_err(LogError::from)
      })
      .transpose()
  }

  async fn delete_meta(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<(), LogError> {
    let mut connection = self.connection.clone();
    let _: usize = connection.del(meta_key(agent_id, session_id)).await?;

    Ok(())
  }
}

fn partition_key(agent_id: &str, session_id: &str, partition: i64) -> String {
  format!(
    "{SESSION_KEY_PREFIX}:{agent_id}:{session_id}:{PART_KEY_SUFFIX}:{partition}"
  )
}

fn current_partition_key(agent_id: &str, session_id: &str) -> String {
  format!(
    "{SESSION_KEY_PREFIX}:{agent_id}:{session_id}:{CURRENT_PARTITION_KEY_SUFFIX}"
  )
}

fn meta_key(agent_id: &str, session_id: &str) -> String {
  format!("{META_KEY_PREFIX}:{agent_id}:{session_id}")
}

fn compress(json: &[u8], level: i32) -> Result<Vec<u8>, LogError> {
  let compressed = zstd::stream::encode_all(json, level).map_err(|error| {
    LogError::Compression {
      message: error.to_string(),
    }
  })?;
  Ok(compressed)
}

fn decompress(compressed: &[u8]) -> Result<LogMessage, LogError> {
  let json = zstd::stream::decode_all(compressed).map_err(|error| {
    LogError::Decompression {
      message: error.to_string(),
    }
  })?;
  let message = serde_json::from_slice(&json)?;
  Ok(message)
}

/// Decompresses a batch of payloads, returning messages in the same order.
fn decompress_all(payloads: Vec<Vec<u8>>) -> Result<Vec<LogMessage>, LogError> {
  let mut messages = Vec::with_capacity(payloads.len());
  for compressed in payloads {
    messages.push(decompress(&compressed)?);
  }
  Ok(messages)
}

#[cfg(test)]
mod tests {
  use redis::Value;
  use redis_test::{MockCmd, MockRedisConnection};

  use super::*;

  /// Small helper: builds storage over a mock connection with a very
  /// low blocking threshold by default, so tests can opt in to the
  /// "large payload -> `spawn_blocking`" path deliberately by
  /// overriding it per test, and stay on the inline path otherwise
  /// by using a generous default.
  fn storage_with_mock(
    mock: MockRedisConnection,
    compress_blocking_threshold: usize,
  ) -> RedisLogStorage<MockRedisConnection> {
    RedisLogStorage::from_parts(
      mock,
      DEFAULT_ZSTD_LEVEL,
      compress_blocking_threshold,
    )
  }

  fn sample_message(id: &str, agent_id: &str, session_id: &str) -> LogMessage {
    LogMessage {
      id: id.to_owned(),
      agent_id: agent_id.to_owned(),
      agent_nickname: "Agent Smith".to_owned(),
      session_id: session_id.to_owned(),
      timestamp: 1_700_000_000_000,
      kind: LogMessageKind::User {
        content: "hello".to_owned(),
      },
    }
  }

  fn sample_condensation(
    id: &str,
    agent_id: &str,
    session_id: &str,
  ) -> LogMessage {
    LogMessage {
      kind: LogMessageKind::Condensation {
        summary: "condensed history".to_owned(),
        model_used: None,
      },
      ..sample_message(id, agent_id, session_id)
    }
  }

  #[tokio::test]
  async fn append_single_message_writes_partition_hash_and_initial_pointer() {
    let agent_id = "agent-1";
    let session_id = "sess-1";
    let message =
      sample_message("01J000000000000000000001", agent_id, session_id);

    let json = serde_json::to_vec(&message).unwrap();
    let payload = compress(&json, DEFAULT_ZSTD_LEVEL).unwrap();

    let pointer_key = current_partition_key(agent_id, session_id);
    let part_key = partition_key(agent_id, session_id, INITIAL_PARTITION);

    let expected_pipeline = redis::pipe()
      .atomic()
      .set(&pointer_key, INITIAL_PARTITION)
      .hset(&part_key, &message.id, &payload)
      .clone();

    let mock = MockRedisConnection::new(vec![
      MockCmd::new(redis::cmd("GET").arg(&pointer_key), Ok(Value::Nil)),
      MockCmd::with_values(
        expected_pipeline,
        Ok(vec![Value::Array(vec![Value::Okay, Value::Int(1)])]),
      ),
    ])
    .assert_all_commands_consumed();

    let storage = storage_with_mock(mock, DEFAULT_COMPRESS_BLOCKING_THRESHOLD);

    let ids = storage
      .append(agent_id, session_id, vec![message.clone()])
      .await
      .expect("append should succeed");

    assert_eq!(ids, vec![message.id]);
  }

  #[tokio::test]
  async fn append_condensation_increments_pointer_and_opens_new_partition() {
    let agent_id = "agent-1";
    let session_id = "sess-1";
    let message =
      sample_condensation("01J000000000000000000002", agent_id, session_id);

    let json = serde_json::to_vec(&message).unwrap();
    let payload = compress(&json, DEFAULT_ZSTD_LEVEL).unwrap();

    let pointer_key = current_partition_key(agent_id, session_id);
    let part_key = partition_key(agent_id, session_id, 3);

    let expected_pipeline = redis::pipe()
      .atomic()
      .incr(&pointer_key, 1)
      .hset(&part_key, &message.id, &payload)
      .clone();

    let mock = MockRedisConnection::new(vec![
      MockCmd::new(redis::cmd("GET").arg(&pointer_key), Ok(2_i64)),
      MockCmd::with_values(
        expected_pipeline,
        Ok(vec![Value::Array(vec![Value::Int(3), Value::Int(1)])]),
      ),
    ])
    .assert_all_commands_consumed();

    let storage = storage_with_mock(mock, DEFAULT_COMPRESS_BLOCKING_THRESHOLD);

    let ids = storage
      .append(agent_id, session_id, vec![message.clone()])
      .await
      .expect("append should succeed");

    assert_eq!(ids, vec![message.id]);
  }

  #[tokio::test]
  async fn append_condensation_without_pointer_opens_partition_one() {
    let agent_id = "agent-1";
    let session_id = "sess-1";
    let message =
      sample_condensation("01J000000000000000000003", agent_id, session_id);

    let json = serde_json::to_vec(&message).unwrap();
    let payload = compress(&json, DEFAULT_ZSTD_LEVEL).unwrap();

    let pointer_key = current_partition_key(agent_id, session_id);
    let part_key = partition_key(agent_id, session_id, 1);

    let expected_pipeline = redis::pipe()
      .atomic()
      .set(&pointer_key, INITIAL_PARTITION)
      .incr(&pointer_key, 1)
      .hset(&part_key, &message.id, &payload)
      .clone();

    let mock = MockRedisConnection::new(vec![
      MockCmd::new(redis::cmd("GET").arg(&pointer_key), Ok(Value::Nil)),
      MockCmd::with_values(
        expected_pipeline,
        Ok(vec![Value::Array(vec![
          Value::Okay,
          Value::Int(1),
          Value::Int(1),
        ])]),
      ),
    ])
    .assert_all_commands_consumed();

    let storage = storage_with_mock(mock, DEFAULT_COMPRESS_BLOCKING_THRESHOLD);

    let ids = storage
      .append(agent_id, session_id, vec![message.clone()])
      .await
      .expect("append should succeed");

    assert_eq!(ids, vec![message.id]);
  }

  #[tokio::test]
  async fn append_mixed_batch_advances_partition_per_condensation() {
    let agent_id = "agent-1";
    let session_id = "sess-1";
    let m1 = sample_message("01J000000000000000000001", agent_id, session_id);
    let m2 =
      sample_condensation("01J000000000000000000002", agent_id, session_id);
    let m3 = sample_message("01J000000000000000000003", agent_id, session_id);

    let payload1 =
      compress(&serde_json::to_vec(&m1).unwrap(), DEFAULT_ZSTD_LEVEL).unwrap();
    let payload2 =
      compress(&serde_json::to_vec(&m2).unwrap(), DEFAULT_ZSTD_LEVEL).unwrap();
    let payload3 =
      compress(&serde_json::to_vec(&m3).unwrap(), DEFAULT_ZSTD_LEVEL).unwrap();

    let pointer_key = current_partition_key(agent_id, session_id);
    let part0 = partition_key(agent_id, session_id, 0);
    let part1 = partition_key(agent_id, session_id, 1);

    let expected_pipeline = redis::pipe()
      .atomic()
      .hset(&part0, &m1.id, &payload1)
      .incr(&pointer_key, 1)
      .hset(&part1, &m2.id, &payload2)
      .hset(&part1, &m3.id, &payload3)
      .clone();

    let mock = MockRedisConnection::new(vec![
      MockCmd::new(redis::cmd("GET").arg(&pointer_key), Ok(0_i64)),
      MockCmd::with_values(
        expected_pipeline,
        Ok(vec![Value::Array(vec![
          Value::Int(1),
          Value::Int(1),
          Value::Int(1),
          Value::Int(1),
        ])]),
      ),
    ])
    .assert_all_commands_consumed();

    let storage = storage_with_mock(mock, DEFAULT_COMPRESS_BLOCKING_THRESHOLD);

    let ids = storage
      .append(
        agent_id,
        session_id,
        vec![m1.clone(), m2.clone(), m3.clone()],
      )
      .await
      .expect("append should succeed");

    assert_eq!(ids, vec![m1.id, m2.id, m3.id]);
  }

  #[tokio::test]
  async fn append_large_message_compresses_via_spawn_blocking_with_identical_result()
   {
    let agent_id = "agent-1";
    let session_id = "sess-1";
    // Long text pushes serialized JSON above a deliberately tiny
    // threshold, forcing the spawn_blocking path.
    let message = LogMessage {
      kind: LogMessageKind::User {
        content: "x".repeat(200),
      },
      ..sample_message("01J000000000000000000002", agent_id, session_id)
    };

    let json = serde_json::to_vec(&message).unwrap();
    assert!(
      json.len() > 16,
      "test payload must exceed the tiny threshold below"
    );
    let payload = compress(&json, DEFAULT_ZSTD_LEVEL).unwrap();

    let pointer_key = current_partition_key(agent_id, session_id);
    let part_key = partition_key(agent_id, session_id, INITIAL_PARTITION);

    let expected_pipeline = redis::pipe()
      .atomic()
      .set(&pointer_key, INITIAL_PARTITION)
      .hset(&part_key, &message.id, &payload)
      .clone();

    let mock = MockRedisConnection::new(vec![
      MockCmd::new(redis::cmd("GET").arg(&pointer_key), Ok(Value::Nil)),
      MockCmd::with_values(
        expected_pipeline,
        Ok(vec![Value::Array(vec![Value::Okay, Value::Int(1)])]),
      ),
    ])
    .assert_all_commands_consumed();

    // Tiny threshold (16 bytes) forces the blocking-task path.
    let storage = storage_with_mock(mock, 16);

    let ids = storage
      .append(agent_id, session_id, vec![message.clone()])
      .await
      .expect("append should succeed even via spawn_blocking");

    assert_eq!(ids, vec![message.id]);
  }

  // --- history -----------------------------------------------------

  #[tokio::test]
  async fn history_reads_active_partition_ordered_by_id() {
    let agent_id = "agent-1";
    let session_id = "sess-1";
    let m1 = sample_message("01J000000000000000000001", agent_id, session_id);
    let m2 = sample_message("01J000000000000000000002", agent_id, session_id);

    let payload1 =
      compress(&serde_json::to_vec(&m1).unwrap(), DEFAULT_ZSTD_LEVEL).unwrap();
    let payload2 =
      compress(&serde_json::to_vec(&m2).unwrap(), DEFAULT_ZSTD_LEVEL).unwrap();

    let pointer_key = current_partition_key(agent_id, session_id);
    let part_key = partition_key(agent_id, session_id, 1);

    // Entries are returned out of order on purpose to prove `history`
    // restores chronological order from the ULID ids.
    let mock = MockRedisConnection::new(vec![
      MockCmd::new(redis::cmd("GET").arg(&pointer_key), Ok(1_i64)),
      MockCmd::new(
        redis::cmd("HGETALL").arg(&part_key),
        Ok(Value::Map(vec![
          (
            Value::BulkString(m2.id.clone().into_bytes()),
            Value::BulkString(payload2),
          ),
          (
            Value::BulkString(m1.id.clone().into_bytes()),
            Value::BulkString(payload1),
          ),
        ])),
      ),
    ])
    .assert_all_commands_consumed();

    let storage = storage_with_mock(mock, DEFAULT_COMPRESS_BLOCKING_THRESHOLD);

    let history = storage
      .history(agent_id, session_id)
      .await
      .expect("history should succeed");

    assert_eq!(
      serde_json::to_value(history).unwrap(),
      serde_json::to_value(vec![m1, m2]).unwrap()
    );
  }

  #[tokio::test]
  async fn history_without_pointer_reads_an_empty_partition() {
    let agent_id = "agent-1";
    let session_id = "sess-1";

    let pointer_key = current_partition_key(agent_id, session_id);
    let part_key = partition_key(agent_id, session_id, INITIAL_PARTITION);

    let mock = MockRedisConnection::new(vec![
      MockCmd::new(redis::cmd("GET").arg(&pointer_key), Ok(Value::Nil)),
      MockCmd::new(
        redis::cmd("HGETALL").arg(&part_key),
        Ok(Value::Map(Vec::new())),
      ),
    ])
    .assert_all_commands_consumed();

    let storage = storage_with_mock(mock, DEFAULT_COMPRESS_BLOCKING_THRESHOLD);

    let history = storage
      .history(agent_id, session_id)
      .await
      .expect("history should succeed");

    assert!(history.is_empty());
  }

  #[tokio::test]
  async fn history_decompresses_large_partition_via_spawn_blocking() {
    let agent_id = "agent-1";
    let session_id = "sess-1";
    let m1 = sample_message("01J000000000000000000001", agent_id, session_id);
    let m2 = sample_message("01J000000000000000000002", agent_id, session_id);

    let payload1 =
      compress(&serde_json::to_vec(&m1).unwrap(), DEFAULT_ZSTD_LEVEL).unwrap();
    let payload2 =
      compress(&serde_json::to_vec(&m2).unwrap(), DEFAULT_ZSTD_LEVEL).unwrap();

    let pointer_key = current_partition_key(agent_id, session_id);
    let part_key = partition_key(agent_id, session_id, INITIAL_PARTITION);

    let mock = MockRedisConnection::new(vec![
      MockCmd::new(redis::cmd("GET").arg(&pointer_key), Ok(Value::Nil)),
      MockCmd::new(
        redis::cmd("HGETALL").arg(&part_key),
        Ok(Value::Map(vec![
          (
            Value::BulkString(m1.id.clone().into_bytes()),
            Value::BulkString(payload1),
          ),
          (
            Value::BulkString(m2.id.clone().into_bytes()),
            Value::BulkString(payload2),
          ),
        ])),
      ),
    ])
    .assert_all_commands_consumed();

    // Tiny threshold (16 bytes) forces the whole decompression batch
    // onto the blocking pool.
    let storage = storage_with_mock(mock, 16);

    let history = storage
      .history(agent_id, session_id)
      .await
      .expect("history should succeed even via spawn_blocking");

    assert_eq!(
      serde_json::to_value(history).unwrap(),
      serde_json::to_value(vec![m1, m2]).unwrap()
    );
  }

  // --- delete_session --------------------------------------------------

  #[tokio::test]
  async fn drain_reads_every_partition_in_order() {
    let agent_id = "agent-1";
    let session_id = "sess-1";
    let m0 = sample_message("01J000000000000000000001", agent_id, session_id);
    let m1 = sample_message("01J000000000000000000002", agent_id, session_id);

    let payload0 =
      compress(&serde_json::to_vec(&m0).unwrap(), DEFAULT_ZSTD_LEVEL).unwrap();
    let payload1 =
      compress(&serde_json::to_vec(&m1).unwrap(), DEFAULT_ZSTD_LEVEL).unwrap();

    let pointer_key = current_partition_key(agent_id, session_id);
    let part0 = partition_key(agent_id, session_id, 0);
    let part1 = partition_key(agent_id, session_id, 1);

    let mock = MockRedisConnection::new(vec![
      MockCmd::new(redis::cmd("GET").arg(&pointer_key), Ok(1_i64)),
      MockCmd::new(
        redis::cmd("HGETALL").arg(&part0),
        Ok(Value::Map(vec![(
          Value::BulkString(m0.id.clone().into_bytes()),
          Value::BulkString(payload0.clone()),
        )])),
      ),
      MockCmd::new(redis::cmd("GET").arg(&pointer_key), Ok(1_i64)),
      MockCmd::new(
        redis::cmd("HGETALL").arg(&part1),
        Ok(Value::Map(vec![(
          Value::BulkString(m1.id.clone().into_bytes()),
          Value::BulkString(payload1.clone()),
        )])),
      ),
      MockCmd::new(redis::cmd("GET").arg(&pointer_key), Ok(1_i64)),
    ])
    .assert_all_commands_consumed();

    let storage = storage_with_mock(mock, DEFAULT_COMPRESS_BLOCKING_THRESHOLD);

    let first = storage
      .drain_next_batch(agent_id, session_id, 0)
      .await
      .expect("first batch should succeed");
    assert_eq!(
      first.map(|(index, batch)| {
        (
          index,
          batch
            .into_iter()
            .map(|message| (message.id, message.payload))
            .collect::<Vec<_>>(),
        )
      }),
      Some((1, vec![(m0.id, payload0)],))
    );

    let second = storage
      .drain_next_batch(agent_id, session_id, 1)
      .await
      .expect("second batch should succeed");
    assert_eq!(
      second.map(|(index, batch)| {
        (
          index,
          batch
            .into_iter()
            .map(|message| (message.id, message.payload))
            .collect::<Vec<_>>(),
        )
      }),
      Some((2, vec![(m1.id, payload1)],))
    );

    assert!(
      storage
        .drain_next_batch(agent_id, session_id, 2)
        .await
        .expect("third batch should succeed")
        .is_none()
    );
  }

  #[tokio::test]
  async fn delete_session_deletes_pointer_and_all_partitions_up_to_current() {
    let agent_id = "agent-1";
    let session_id = "sess-1";

    let pointer_key = current_partition_key(agent_id, session_id);
    let expected_pipeline = redis::pipe()
      .atomic()
      .del(&pointer_key)
      .del(partition_key(agent_id, session_id, 0))
      .del(partition_key(agent_id, session_id, 1))
      .del(partition_key(agent_id, session_id, 2))
      .clone();

    let mock = MockRedisConnection::new(vec![
      MockCmd::new(redis::cmd("GET").arg(&pointer_key), Ok(2_i64)),
      MockCmd::with_values(
        expected_pipeline,
        Ok(vec![Value::Array(vec![
          Value::Int(1),
          Value::Int(1),
          Value::Int(1),
          Value::Int(1),
        ])]),
      ),
    ])
    .assert_all_commands_consumed();

    let storage = storage_with_mock(mock, DEFAULT_COMPRESS_BLOCKING_THRESHOLD);

    storage
      .delete_session(agent_id, session_id)
      .await
      .expect("delete_session should succeed");
  }

  #[tokio::test]
  async fn delete_session_without_pointer_deletes_initial_partition() {
    let agent_id = "agent-1";
    let session_id = "sess-1";

    let pointer_key = current_partition_key(agent_id, session_id);
    let expected_pipeline = redis::pipe()
      .atomic()
      .del(&pointer_key)
      .del(partition_key(agent_id, session_id, INITIAL_PARTITION))
      .clone();

    let mock = MockRedisConnection::new(vec![
      MockCmd::new(redis::cmd("GET").arg(&pointer_key), Ok(Value::Nil)),
      MockCmd::with_values(
        expected_pipeline,
        Ok(vec![Value::Array(vec![Value::Int(1), Value::Int(1)])]),
      ),
    ])
    .assert_all_commands_consumed();

    let storage = storage_with_mock(mock, DEFAULT_COMPRESS_BLOCKING_THRESHOLD);

    storage
      .delete_session(agent_id, session_id)
      .await
      .expect("delete_session should succeed");
  }

  // --- SessionMetaStore --------------------------------------------

  #[tokio::test]
  async fn put_meta_stores_serialized_json_under_meta_key() {
    let meta = SessionMeta {
      agent_id: "agent-1".to_owned(),
      agent_nickname: "Agent Smith".to_owned(),
      session_id: "sess-1".to_owned(),
      created_at: 1_700_000_000_000,
      closed_at: None,
      workflow_id: None,
    };
    let serialized = serde_json::to_string(&meta).unwrap();
    let key = meta_key(&meta.agent_id, &meta.session_id);

    let mock = MockRedisConnection::new(vec![MockCmd::new(
      redis::cmd("SET").arg(&key).arg(&serialized),
      Ok("OK"),
    )])
    .assert_all_commands_consumed();

    let storage = storage_with_mock(mock, DEFAULT_COMPRESS_BLOCKING_THRESHOLD);

    storage
      .put_meta(&meta)
      .await
      .expect("put_meta should succeed");
  }

  #[tokio::test]
  async fn get_meta_returns_none_when_missing() {
    let agent_id = "agent-1";
    let session_id = "sess-1";
    let key = meta_key(agent_id, session_id);

    let mock = MockRedisConnection::new(vec![MockCmd::new(
      redis::cmd("GET").arg(&key),
      Ok(Value::Nil),
    )])
    .assert_all_commands_consumed();

    let storage = storage_with_mock(mock, DEFAULT_COMPRESS_BLOCKING_THRESHOLD);

    let result = storage
      .get_meta(agent_id, session_id)
      .await
      .expect("get_meta should succeed");

    assert_eq!(result, None);
  }

  #[tokio::test]
  async fn delete_meta_removes_the_key() {
    let agent_id = "agent-1";
    let session_id = "sess-1";
    let key = meta_key(agent_id, session_id);

    let mock = MockRedisConnection::new(vec![MockCmd::new(
      redis::cmd("DEL").arg(&key),
      Ok(1),
    )])
    .assert_all_commands_consumed();

    let storage = storage_with_mock(mock, DEFAULT_COMPRESS_BLOCKING_THRESHOLD);

    storage
      .delete_meta(agent_id, session_id)
      .await
      .expect("delete_meta should succeed");
  }
}
