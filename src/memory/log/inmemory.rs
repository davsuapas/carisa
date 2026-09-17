//! Storage doubles available only while testing.

use std::{collections::HashMap, sync::Mutex};

use async_trait::async_trait;

use super::{
  error::LogError,
  storage::{ColdStorage, LogStorage, SessionMetaStore},
  types::{
    LogMessage, LogMessageKind, MessageId, SessionMeta, StoredLogMessage,
  },
};

type SessionKey = (String, String);
type PartitionedMessages = Vec<Vec<LogMessage>>;
type MessageMap = HashMap<SessionKey, PartitionedMessages>;
type StoredMessageMap = HashMap<SessionKey, Vec<Vec<StoredLogMessage>>>;
type MetaMap = HashMap<SessionKey, SessionMeta>;

/// In-memory active storage for deterministic tests.
///
/// Mirrors the Redis backend: a condensation message closes the current
/// partition and opens a new one whose first entry is that condensation.
#[derive(Debug, Default)]
pub struct InMemoryLogStorage {
  messages: Mutex<MessageMap>,
}

impl InMemoryLogStorage {
  /// Creates an empty storage.
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }
}

#[async_trait]
impl LogStorage for InMemoryLogStorage {
  async fn append(
    &self,
    _agent_id: &str,
    _session_id: &str,
    messages: Vec<LogMessage>,
  ) -> Result<Vec<MessageId>, LogError> {
    let ids = messages.iter().map(|message| message.id.clone()).collect();
    if messages.is_empty() {
      return Ok(ids);
    }

    let key = session_key(_agent_id, _session_id);
    let mut stored = self.messages.lock().map_err(|_| LogError::Internal {
      message: "message lock poisoned".to_owned(),
    })?;
    let partitions = stored.entry(key).or_default();

    for message in messages {
      let is_condensation =
        matches!(&message.kind, LogMessageKind::Condensation { .. });
      if is_condensation || partitions.is_empty() {
        partitions.push(Vec::new());
      }
      let partition = partitions.last_mut().ok_or_else(|| LogError::Internal {
        message: "active session has no partition".to_owned(),
      })?;
      partition.push(message);
    }

    drop(stored);
    Ok(ids)
  }

  async fn history(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<Vec<LogMessage>, LogError> {
    let key = session_key(agent_id, session_id);
    let stored = self.messages.lock().map_err(|_| LogError::Internal {
      message: "message lock poisoned".to_owned(),
    })?;
    let messages = stored
      .get(&key)
      .and_then(|partitions| partitions.last())
      .cloned()
      .unwrap_or_default();
    drop(stored);
    Ok(messages)
  }

  async fn drain_next_batch(
    &self,
    agent_id: &str,
    session_id: &str,
    batch_index: usize,
  ) -> Result<Option<(usize, Vec<StoredLogMessage>)>, LogError> {
    let key = session_key(agent_id, session_id);
    let stored = self.messages.lock().map_err(|_| LogError::Internal {
      message: "message lock poisoned".to_owned(),
    })?;
    let partitions = stored.get(&key).cloned().unwrap_or_default();
    drop(stored);

    let batch = partitions.get(batch_index).cloned().map(|messages| {
      messages
        .into_iter()
        .map(|message| {
          let id = message.id.clone();
          let payload = serde_json::to_vec(&message)?;
          Ok(StoredLogMessage { id, payload })
        })
        .collect::<Result<Vec<_>, serde_json::Error>>()
    });
    batch.map_or_else(
      || Ok(None),
      |batch| {
        batch
          .map(|batch| Some((batch_index.saturating_add(1), batch)))
          .map_err(LogError::from)
      },
    )
  }

  async fn delete_session(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<(), LogError> {
    let key = session_key(agent_id, session_id);
    let mut stored = self.messages.lock().map_err(|_| LogError::Internal {
      message: "message lock poisoned".to_owned(),
    })?;
    stored.remove(&key);
    drop(stored);
    Ok(())
  }
}

/// In-memory cold storage with idempotent appends and archived metadata.
#[derive(Debug, Default)]
pub struct InMemoryColdStorage {
  messages: Mutex<StoredMessageMap>,
  metadata: Mutex<MetaMap>,
}

impl InMemoryColdStorage {
  /// Creates an empty cold storage.
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }

  /// Returns messages stored for a test session.
  ///
  /// # Errors
  ///
  /// Returns an error if the internal mutex is poisoned.
  pub fn messages(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<Vec<Vec<u8>>, LogError> {
    let key = session_key(agent_id, session_id);
    let stored = self.messages.lock().map_err(|_| LogError::Internal {
      message: "message lock poisoned".to_owned(),
    })?;
    let stored_messages = stored
      .get(&key)
      .map(|partitions| partitions.concat())
      .unwrap_or_default();
    drop(stored);
    Ok(
      stored_messages
        .into_iter()
        .map(|message| message.payload)
        .collect(),
    )
  }
}

#[async_trait]
impl ColdStorage for InMemoryColdStorage {
  async fn append(
    &self,
    agent_id: &str,
    session_id: &str,
    messages: Vec<StoredLogMessage>,
  ) -> Result<(), LogError> {
    let key = session_key(agent_id, session_id);
    let mut stored = self.messages.lock().map_err(|_| LogError::Internal {
      message: "message lock poisoned".to_owned(),
    })?;
    let session = stored.entry(key).or_default();
    if session.is_empty() {
      session.push(Vec::new());
    }
    let partition = session.first_mut().ok_or_else(|| LogError::Internal {
      message: "cold session has no partition".to_owned(),
    })?;
    for message in messages {
      if partition.iter().all(|stored| stored.id != message.id) {
        partition.push(message);
      }
    }
    drop(stored);
    Ok(())
  }

  async fn put_meta(&self, meta: &SessionMeta) -> Result<(), LogError> {
    let key = session_key(&meta.agent_id, &meta.session_id);
    let mut stored = self.metadata.lock().map_err(|_| LogError::Internal {
      message: "metadata lock poisoned".to_owned(),
    })?;
    stored.insert(key, meta.clone());
    drop(stored);
    Ok(())
  }

  async fn get_meta(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<Option<SessionMeta>, LogError> {
    let key = session_key(agent_id, session_id);
    let stored = self.metadata.lock().map_err(|_| LogError::Internal {
      message: "metadata lock poisoned".to_owned(),
    })?;
    let meta = stored.get(&key).cloned();
    drop(stored);
    Ok(meta)
  }
}

/// In-memory session metadata storage.
#[derive(Debug, Default)]
pub struct InMemorySessionMetaStore {
  metadata: Mutex<MetaMap>,
}

impl InMemorySessionMetaStore {
  /// Creates an empty metadata store.
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }
}

#[async_trait]
impl SessionMetaStore for InMemorySessionMetaStore {
  async fn put_meta(&self, meta: &SessionMeta) -> Result<(), LogError> {
    let key = session_key(&meta.agent_id, &meta.session_id);
    let mut stored = self.metadata.lock().map_err(|_| LogError::Internal {
      message: "metadata lock poisoned".to_owned(),
    })?;
    stored.insert(key, meta.clone());
    drop(stored);
    Ok(())
  }

  async fn get_meta(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<Option<SessionMeta>, LogError> {
    let key = session_key(agent_id, session_id);
    let stored = self.metadata.lock().map_err(|_| LogError::Internal {
      message: "metadata lock poisoned".to_owned(),
    })?;
    let metadata = stored.get(&key).cloned();
    drop(stored);
    Ok(metadata)
  }

  async fn delete_meta(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<(), LogError> {
    let key = session_key(agent_id, session_id);
    let mut stored = self.metadata.lock().map_err(|_| LogError::Internal {
      message: "metadata lock poisoned".to_owned(),
    })?;
    stored.remove(&key);
    drop(stored);
    Ok(())
  }
}

fn session_key(agent_id: &str, session_id: &str) -> SessionKey {
  (agent_id.to_owned(), session_id.to_owned())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn message(id: &str) -> LogMessage {
    LogMessage {
      id: id.to_owned(),
      agent_id: "agent".to_owned(),
      agent_nickname: "Agent".to_owned(),
      session_id: "session".to_owned(),
      timestamp: 1,
      kind: LogMessageKind::User {
        content: id.to_owned(),
      },
    }
  }

  #[tokio::test]
  async fn log_storage_preserves_order() {
    let storage = InMemoryLogStorage::new();
    storage
      .append("agent", "session", vec![message("one"), message("two")])
      .await
      .expect("append should work");

    let history = storage
      .history("agent", "session")
      .await
      .expect("history should work");

    assert_eq!(
      history
        .iter()
        .map(|item| item.id.as_str())
        .collect::<Vec<_>>(),
      ["one", "two"]
    );
    storage
      .delete_session("agent", "session")
      .await
      .expect("delete should work");
    assert!(
      storage
        .history("agent", "session")
        .await
        .expect("history should work")
        .is_empty()
    );
  }

  #[tokio::test]
  async fn drain_next_batch_returns_one_batch_per_partition() {
    let storage = InMemoryLogStorage::new();
    storage
      .append(
        "agent",
        "session",
        vec![
          message("one"),
          LogMessage {
            kind: LogMessageKind::Condensation {
              summary: "summary".to_owned(),
              model_used: None,
            },
            ..message("summary")
          },
          message("two"),
        ],
      )
      .await
      .expect("append should work");

    let first = storage
      .drain_next_batch("agent", "session", 0)
      .await
      .expect("first batch should work");
    assert_eq!(
      first.map(|(index, batch)| {
        (
          index,
          batch.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
        )
      }),
      Some((1, vec!["one".to_owned()]))
    );

    let second = storage
      .drain_next_batch("agent", "session", 1)
      .await
      .expect("second batch should work");
    assert_eq!(
      second.map(|(index, batch)| {
        (
          index,
          batch.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
        )
      }),
      Some((2, vec!["summary".to_owned(), "two".to_owned()]))
    );

    assert!(
      storage
        .drain_next_batch("agent", "session", 2)
        .await
        .expect("third batch should work")
        .is_none()
    );
  }

  #[tokio::test]
  async fn cold_storage_is_idempotent_and_metadata_round_trips() {
    let cold = InMemoryColdStorage::new();
    cold
      .append(
        "agent",
        "session",
        vec![StoredLogMessage {
          id: "one".to_owned(),
          payload: serde_json::to_vec(&message("one")).expect("serialize"),
        }],
      )
      .await
      .expect("append should work");
    cold
      .append(
        "agent",
        "session",
        vec![StoredLogMessage {
          id: "one".to_owned(),
          payload: serde_json::to_vec(&message("one")).expect("serialize"),
        }],
      )
      .await
      .expect("append should work");
    let payloads = cold.messages("agent", "session").expect("read should work");
    assert_eq!(payloads.len(), 1);
    assert!(
      !payloads
        .first()
        .expect("one payload should exist")
        .is_empty()
    );

    let archived = SessionMeta {
      agent_id: "agent".to_owned(),
      agent_nickname: "Agent".to_owned(),
      session_id: "session".to_owned(),
      created_at: 1,
      closed_at: Some(2),
      workflow_id: None,
    };
    cold
      .put_meta(&archived)
      .await
      .expect("cold meta should save");
    assert_eq!(
      cold
        .get_meta("agent", "session")
        .await
        .expect("cold meta should load"),
      Some(archived)
    );

    let metadata = InMemorySessionMetaStore::new();
    let meta = SessionMeta {
      agent_id: "agent".to_owned(),
      agent_nickname: "Agent".to_owned(),
      session_id: "session".to_owned(),
      created_at: 1,
      closed_at: Some(2),
      workflow_id: None,
    };
    metadata
      .put_meta(&meta)
      .await
      .expect("metadata should save");
    assert_eq!(
      metadata
        .get_meta("agent", "session")
        .await
        .expect("metadata should load"),
      Some(meta.clone())
    );
    metadata
      .delete_meta("agent", "session")
      .await
      .expect("metadata should delete");
    assert!(
      metadata
        .get_meta("agent", "session")
        .await
        .expect("metadata should load")
        .is_none()
    );
  }
}
