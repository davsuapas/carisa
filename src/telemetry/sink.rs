//! Sinks, retry policy and the in-memory test sink.
//!
//! A sink is any type that implements [`TraceSink`]: it receives batches of
//! [`TraceRecord`]s and persists them. The worker fans a batch out to every
//! registered sink at the same time, so a sink that returns an error never
//! prevents the others from receiving the data.
//!
//! # Delivery semantics
//!
//! Delivery is *at-least-once* per sink. When a sink reports a
//! [`SinkError::Transient`] failure the whole batch is retried, even if the
//! sink had already written part of it. A real sink must therefore be
//! idempotent or deduplicate by [`TraceRecord::seq`]. The worker cannot do it
//! for you: it has no way to know how much a sink wrote before failing.
//!
//! # Writing a sink
//!
//! - Keep [`TraceSink::write_batch`] fast. The worker processes one batch at
//!   a time, so a slow sink delays every other sink and, with a small channel,
//!   increases the number of dropped records.
//! - Classify errors honestly: return [`SinkError::Transient`] for timeouts,
//!   connection resets and rate limits (they are retried), and
//!   [`SinkError::Permanent`] for authentication or schema errors (they are
//!   not).
//! - Never emit `persist_*!` events from inside `write_batch`. They would be
//!   enqueued again, amplifying the load, and they would carry no agent
//!   context. Use plain `tracing` calls if you need to log.

use std::collections::VecDeque;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use super::DEFAULT_MEMORY_SINK_CAPACITY;
use super::record::TraceRecord;
use super::runtime::TelemetryError;

/// Default number of attempts (including the first one) of a retried batch.
const DEFAULT_MAX_ATTEMPTS: u32 = 3;
/// Default delay before the second attempt.
const DEFAULT_INITIAL_BACKOFF: Duration = Duration::from_millis(100);
/// Default upper bound for the delay between attempts.
const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(5);
/// Default factor applied to the delay after every failed attempt.
const DEFAULT_BACKOFF_MULTIPLIER: u32 = 2;

/// Failure reported by a [`TraceSink`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SinkError {
  /// A recoverable failure. The worker retries the batch.
  #[error("transient sink error: {0}")]
  Transient(String),
  /// A failure that will not be fixed by retrying.
  #[error("permanent sink error: {0}")]
  Permanent(String),
}

/// Classification used by the [`MemorySink`] failure injection helpers.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
  clippy::redundant_pub_crate,
  reason = "re-exported crate-wide by the module root"
)]
pub(crate) enum FailKind {
  /// A failure the worker retries.
  Transient,
  /// A failure the worker does not retry.
  Permanent,
}

#[cfg(test)]
impl From<FailKind> for SinkError {
  fn from(kind: FailKind) -> Self {
    match kind {
      FailKind::Transient => Self::Transient("injected transient".to_owned()),
      FailKind::Permanent => Self::Permanent("injected permanent".to_owned()),
    }
  }
}

/// Destination of the persisted records.
///
/// Implement it to store records somewhere else (a file, a database, a
/// remote service). See the [module documentation](crate::telemetry) for the
/// rules a sink must follow.
#[async_trait]
pub trait TraceSink: Send + Sync {
  /// Returns the sink name, used in diagnostics and internal error events.
  fn name(&self) -> &str;

  /// Writes one batch of records.
  ///
  /// # Errors
  ///
  /// Returns [`SinkError::Transient`] when the batch can be retried and
  /// [`SinkError::Permanent`] when it cannot. A returned error means the
  /// whole batch must be considered unsent.
  async fn write_batch(&self, batch: &[TraceRecord]) -> Result<(), SinkError>;
}

/// Configuration of the retry loop used by the worker.
///
/// The values are validated at construction, so an instance can only exist in
/// a valid state; [`RetryPolicy::default`] is valid by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
  max_attempts: u32,
  initial_backoff: Duration,
  max_backoff: Duration,
  multiplier: u32,
}

impl RetryPolicy {
  /// Builds a policy from explicit values.
  ///
  /// # Errors
  ///
  /// Returns [`TelemetryError::InvalidConfig`] when `max_attempts` or
  /// `multiplier` is zero, or when `max_backoff` is smaller than
  /// `initial_backoff`.
  pub fn new(
    max_attempts: u32,
    initial_backoff: Duration,
    max_backoff: Duration,
    multiplier: u32,
  ) -> Result<Self, TelemetryError> {
    if max_attempts == 0 {
      return Err(TelemetryError::InvalidConfig(
        "retry max_attempts must be greater than zero",
      ));
    }

    if multiplier == 0 {
      return Err(TelemetryError::InvalidConfig(
        "retry multiplier must be greater than zero",
      ));
    }

    if max_backoff < initial_backoff {
      return Err(TelemetryError::InvalidConfig(
        "retry max_backoff must not be smaller than initial_backoff",
      ));
    }

    Ok(Self {
      max_attempts,
      initial_backoff,
      max_backoff,
      multiplier,
    })
  }

  /// Returns the maximum number of attempts, including the first one.
  #[must_use]
  pub const fn max_attempts(&self) -> u32 {
    self.max_attempts
  }

  /// Returns the delay before the second attempt.
  #[must_use]
  pub const fn initial_backoff(&self) -> Duration {
    self.initial_backoff
  }

  /// Returns the upper bound of the delay between attempts.
  #[must_use]
  pub const fn max_backoff(&self) -> Duration {
    self.max_backoff
  }

  /// Returns the factor applied to the delay after a failed attempt.
  #[must_use]
  pub const fn multiplier(&self) -> u32 {
    self.multiplier
  }
}

impl Default for RetryPolicy {
  fn default() -> Self {
    Self {
      max_attempts: DEFAULT_MAX_ATTEMPTS,
      initial_backoff: DEFAULT_INITIAL_BACKOFF,
      max_backoff: DEFAULT_MAX_BACKOFF,
      multiplier: DEFAULT_BACKOFF_MULTIPLIER,
    }
  }
}

/// Writes a batch to a sink, retrying transient failures.
///
/// Retries use an exponential backoff that starts at
/// [`RetryPolicy::initial_backoff`] and is capped at
/// [`RetryPolicy::max_backoff`]. A [`SinkError::Permanent`] failure is
/// returned immediately, without retrying.
///
/// # Errors
///
/// Returns the last [`SinkError`] reported by the sink once the attempts are
/// exhausted, or the permanent error on the first failure of that kind.
pub(super) async fn write_with_retry(
  sink: &dyn TraceSink,
  batch: &[TraceRecord],
  policy: &RetryPolicy,
) -> Result<(), SinkError> {
  let mut backoff = policy.initial_backoff;
  let mut attempt: u32 = 1;

  loop {
    match sink.write_batch(batch).await {
      Ok(()) => return Ok(()),
      Err(SinkError::Permanent(message)) => {
        return Err(SinkError::Permanent(message));
      }
      Err(SinkError::Transient(message)) => {
        if attempt >= policy.max_attempts {
          return Err(SinkError::Transient(message));
        }

        tokio::time::sleep(backoff).await;
        backoff = backoff
          .saturating_mul(policy.multiplier)
          .min(policy.max_backoff);
        attempt = attempt.saturating_add(1);
      }
    }
  }
}

/// In-memory sink, intended for development, tests and small bursts.
///
/// Every clone shares the same store, so a handle obtained from
/// [`crate::telemetry::Telemetry::in_memory`] observes what the worker
/// writes.
///
/// The sink is bounded by default: once the configured capacity is exceeded
/// the oldest records are discarded to make room. Discarding is not an error,
/// it is the documented behaviour of the bounded store.
#[derive(Debug, Clone)]
pub struct MemorySink {
  store: Arc<Mutex<VecDeque<TraceRecord>>>,
  capacity: Option<usize>,
  #[cfg(test)]
  call_count: Arc<AtomicU64>,
  #[cfg(test)]
  failure: Arc<Mutex<Failure>>,
}

/// State of the failure injection helpers.
#[cfg(test)]
#[derive(Debug, Default)]
enum Failure {
  /// Every call succeeds.
  #[default]
  None,
  /// The next `remaining` calls fail.
  Next {
    /// Calls left to fail.
    remaining: u32,
    /// Kind of the injected error.
    kind: FailKind,
  },
  /// Every call fails.
  Always(FailKind),
}

/// Locks a mutex, recovering the data even if a previous holder panicked.
///
/// A poisoned mutex must not abort telemetry: the store is a plain
/// `VecDeque`, so the data behind the guard is still consistent once the
/// lock is taken.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
  mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl MemorySink {
  /// Creates a sink bounded by the default capacity: it keeps the newest
  /// 10 000 records and discards the oldest ones beyond that.
  #[must_use]
  pub fn new() -> Self {
    Self::with_capacity(DEFAULT_MEMORY_SINK_CAPACITY)
  }

  /// Creates a sink that keeps at most `max` records.
  ///
  /// When the store is full, the oldest record is discarded before the new
  /// one is appended. A capacity of `0` is valid and stores nothing: every
  /// write succeeds and no record is retained.
  #[must_use]
  pub fn with_capacity(max: usize) -> Self {
    Self::from_capacity(Some(max))
  }

  /// Creates a sink that keeps every record it receives.
  ///
  /// Prefer [`MemorySink::new`] outside tests: an unbounded store grows
  /// without limit if the throughput of the producer exceeds the capacity of
  /// the process.
  #[must_use]
  pub fn unbounded() -> Self {
    Self::from_capacity(None)
  }

  fn from_capacity(capacity: Option<usize>) -> Self {
    Self {
      store: Arc::new(Mutex::new(VecDeque::new())),
      capacity,
      #[cfg(test)]
      call_count: Arc::new(AtomicU64::new(0)),
      #[cfg(test)]
      failure: Arc::new(Mutex::new(Failure::None)),
    }
  }

  /// Returns a snapshot of the stored records, oldest first.
  ///
  /// The store keeps mutating, so the records are cloned into a new `Vec`.
  #[must_use]
  pub fn records(&self) -> Vec<TraceRecord> {
    lock(&self.store).iter().cloned().collect()
  }

  /// Returns how many records are currently stored.
  #[must_use]
  pub fn len(&self) -> usize {
    lock(&self.store).len()
  }

  /// Returns `true` when no record is stored.
  #[must_use]
  pub fn is_empty(&self) -> bool {
    lock(&self.store).is_empty()
  }

  /// Removes every stored record.
  pub fn clear(&self) {
    lock(&self.store).clear();
  }

  /// Returns how many times [`TraceSink::write_batch`] has been called.
  ///
  /// Retries count as separate calls.
  #[cfg(test)]
  #[must_use]
  pub(crate) fn call_count(&self) -> u64 {
    self.call_count.load(Ordering::Relaxed)
  }

  /// Makes the next `times` calls fail with the given kind.
  #[cfg(test)]
  pub(crate) fn fail_next(&self, times: u32, kind: FailKind) {
    *lock(&self.failure) = Failure::Next {
      remaining: times,
      kind,
    };
  }

  /// Makes every subsequent call fail with the given kind.
  #[cfg(test)]
  pub(crate) fn fail_always(&self, kind: FailKind) {
    *lock(&self.failure) = Failure::Always(kind);
  }

  /// Returns the failure to report now, if any, and advances the counter.
  #[cfg(test)]
  fn take_failure(&self) -> Option<SinkError> {
    let mut failure = lock(&self.failure);

    match &mut *failure {
      Failure::None => None,
      Failure::Always(kind) => Some(SinkError::from(*kind)),
      Failure::Next { remaining, kind } => {
        if *remaining == 0 {
          return None;
        }

        *remaining = remaining.saturating_sub(1);
        Some(SinkError::from(*kind))
      }
    }
  }
}

impl Default for MemorySink {
  fn default() -> Self {
    Self::new()
  }
}

#[async_trait]
impl TraceSink for MemorySink {
  fn name(&self) -> &'static str {
    "memory"
  }

  async fn write_batch(&self, batch: &[TraceRecord]) -> Result<(), SinkError> {
    #[cfg(test)]
    {
      self.call_count.fetch_add(1, Ordering::Relaxed);

      if let Some(error) = self.take_failure() {
        return Err(error);
      }
    }

    let Some(capacity) = self.capacity else {
      lock(&self.store).extend(batch.iter().cloned());
      return Ok(());
    };

    if capacity == 0 {
      return Ok(());
    }

    let mut store = lock(&self.store);
    for record in batch {
      while store.len() >= capacity {
        store.pop_front();
      }

      store.push_back(record.clone());
    }

    drop(store);
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use std::sync::Arc;
  use std::time::Duration;

  use rstest::rstest;

  use super::{
    FailKind, MemorySink, RetryPolicy, SinkError, TraceSink, write_with_retry,
  };
  use crate::telemetry::{TraceLevel, TraceRecord};

  fn record(seq: u64) -> TraceRecord {
    let mut record = TraceRecord::new(
      TraceLevel::Info,
      std::borrow::Cow::Borrowed("tests"),
      format!("record {seq}"),
      Vec::new(),
    );
    record.seq = seq;
    record
  }

  fn batch(seqs: &[u64]) -> Vec<TraceRecord> {
    seqs.iter().copied().map(record).collect()
  }

  #[tokio::test]
  async fn memory_sink_writes_and_reads_in_order() {
    // Arrange
    let sink = MemorySink::new();
    let batch = batch(&[1, 2, 3]);

    // Act
    sink.write_batch(&batch).await.expect("write succeeds");

    // Assert
    let seqs: Vec<u64> = sink.records().iter().map(|r| r.seq).collect();
    assert_eq!(seqs, vec![1, 2, 3]);
    assert_eq!(sink.len(), 3);
    assert!(!sink.is_empty());
  }

  #[tokio::test]
  async fn empty_store_returns_an_empty_snapshot() {
    // Arrange
    let sink = MemorySink::new();

    // Act
    let stored = sink.records();

    // Assert
    assert_eq!(stored.len(), 0);
    assert!(sink.is_empty());
    assert_eq!(sink.len(), 0);
  }

  #[tokio::test]
  async fn capacity_of_zero_stores_nothing() {
    // Arrange
    let sink = MemorySink::with_capacity(0);

    // Act
    sink
      .write_batch(&batch(&[1, 2]))
      .await
      .expect("write succeeds");

    // Assert
    assert!(sink.is_empty());
    assert_eq!(sink.call_count(), 1);
  }

  #[tokio::test]
  async fn capacity_boundary_at_the_limit_keeps_all_records() {
    // Arrange
    let sink = MemorySink::with_capacity(3);

    // Act
    sink
      .write_batch(&batch(&[1, 2, 3]))
      .await
      .expect("write succeeds");

    // Assert
    assert_eq!(sink.len(), 3);
  }

  #[tokio::test]
  async fn capacity_boundary_one_over_the_limit_drops_the_oldest() {
    // Arrange
    let sink = MemorySink::with_capacity(3);

    // Act
    sink
      .write_batch(&batch(&[1, 2, 3, 4]))
      .await
      .expect("write succeeds");

    // Assert
    let seqs: Vec<u64> = sink.records().iter().map(|r| r.seq).collect();
    assert_eq!(seqs, vec![2, 3, 4]);
  }

  #[tokio::test]
  async fn unbounded_sink_never_drops_records() {
    // Arrange
    let sink = MemorySink::unbounded();
    let many = batch(&[1, 2, 3, 4, 5]);
    let expected = many.len();

    // Act
    sink.write_batch(&many).await.expect("write succeeds");

    // Assert
    assert_eq!(sink.len(), expected);
  }

  #[tokio::test]
  async fn clear_removes_every_record() {
    // Arrange
    let sink = MemorySink::new();
    sink
      .write_batch(&batch(&[1, 2]))
      .await
      .expect("write succeeds");

    // Act
    sink.clear();

    // Assert
    assert!(sink.is_empty());
  }

  #[tokio::test]
  async fn clones_share_the_same_store() {
    // Arrange
    let sink = MemorySink::new();
    let clone = sink.clone();

    // Act
    clone
      .write_batch(&batch(&[7]))
      .await
      .expect("write succeeds");

    // Assert
    assert_eq!(sink.len(), 1);
    assert_eq!(sink.records().first().map(|r| r.seq), Some(7));
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
  async fn concurrent_writes_keep_every_record() {
    // Arrange
    let sink = MemorySink::unbounded();
    let writers: u64 = 4;
    let per_writer: u64 = 50;

    // Act
    let handles: Vec<_> = (0..writers)
      .map(|writer| {
        let sink = sink.clone();
        tokio::spawn(async move {
          for index in 0..per_writer {
            let seq = writer.saturating_mul(per_writer).saturating_add(index);
            sink
              .write_batch(&[record(seq)])
              .await
              .expect("write succeeds");
          }
        })
      })
      .collect();

    for handle in handles {
      handle.await.expect("writer task finishes");
    }

    // Assert
    let expected = usize::try_from(writers.saturating_mul(per_writer))
      .expect("fits in usize");
    assert_eq!(sink.len(), expected);
  }

  #[tokio::test]
  async fn fail_next_fails_exactly_n_times_then_succeeds() {
    // Arrange
    let sink = MemorySink::new();
    sink.fail_next(2, FailKind::Transient);

    // Act
    let first = sink.write_batch(&[]).await;
    let second = sink.write_batch(&[]).await;
    let third = sink.write_batch(&[]).await;

    // Assert
    assert_eq!(
      first,
      Err(SinkError::Transient("injected transient".to_owned()))
    );
    assert_eq!(
      second,
      Err(SinkError::Transient("injected transient".to_owned()))
    );
    assert_eq!(third, Ok(()));
    assert_eq!(sink.call_count(), 3);
  }

  #[tokio::test]
  async fn fail_always_fails_every_time() {
    // Arrange
    let sink = MemorySink::new();
    sink.fail_always(FailKind::Permanent);

    // Act
    let first = sink.write_batch(&[]).await;
    let second = sink.write_batch(&[]).await;

    // Assert
    assert!(matches!(first, Err(SinkError::Permanent(_))));
    assert!(matches!(second, Err(SinkError::Permanent(_))));
    assert_eq!(sink.call_count(), 2);
  }

  #[rstest]
  #[case(0, 100, 500, 2)]
  #[case(3, 100, 500, 0)]
  #[case(3, 500, 100, 2)]
  fn retry_policy_new_rejects_invalid_values(
    #[case] max_attempts: u32,
    #[case] initial_ms: u64,
    #[case] max_ms: u64,
    #[case] multiplier: u32,
  ) {
    // Arrange
    let initial = Duration::from_millis(initial_ms);
    let max = Duration::from_millis(max_ms);

    // Act
    let policy = RetryPolicy::new(max_attempts, initial, max, multiplier);

    // Assert
    assert!(policy.is_err());
  }

  #[test]
  fn retry_policy_new_accepts_equal_initial_and_max_backoff() {
    // Arrange
    let backoff = Duration::from_millis(250);

    // Act
    let policy = RetryPolicy::new(2, backoff, backoff, 3);

    // Assert
    assert!(policy.is_ok());
  }

  #[test]
  fn retry_policy_default_has_the_documented_values() {
    // Arrange

    // Act
    let policy = RetryPolicy::default();

    // Assert
    assert_eq!(policy.max_attempts(), 3);
    assert_eq!(policy.initial_backoff(), Duration::from_millis(100));
    assert_eq!(policy.max_backoff(), Duration::from_secs(5));
    assert_eq!(policy.multiplier(), 2);
  }

  #[tokio::test]
  async fn permanent_failure_is_not_retried() {
    // Arrange
    let sink = MemorySink::new();
    sink.fail_always(FailKind::Permanent);
    let policy = RetryPolicy::default();

    // Act
    let result = write_with_retry(&sink, &batch(&[1]), &policy).await;

    // Assert
    assert!(matches!(result, Err(SinkError::Permanent(_))));
    assert_eq!(sink.call_count(), 1);
  }

  #[rstest]
  #[case(1)]
  #[case(3)]
  #[case(5)]
  #[tokio::test]
  async fn transient_failure_is_retried_up_to_max_attempts(
    #[case] max_attempts: u32,
  ) {
    // Arrange
    let sink = MemorySink::new();
    sink.fail_always(FailKind::Transient);
    let policy = RetryPolicy::new(
      max_attempts,
      Duration::from_millis(10),
      Duration::from_millis(10),
      1,
    )
    .expect("valid policy");

    // Act
    let result = write_with_retry(&sink, &batch(&[1]), &policy).await;

    // Assert
    assert!(matches!(result, Err(SinkError::Transient(_))));
    assert_eq!(sink.call_count(), u64::from(max_attempts));
  }

  #[tokio::test]
  async fn success_at_the_second_attempt_stops_retrying() {
    // Arrange
    let sink = MemorySink::new();
    sink.fail_next(1, FailKind::Transient);
    let policy = RetryPolicy::new(
      4,
      Duration::from_millis(10),
      Duration::from_millis(10),
      1,
    )
    .expect("valid policy");

    // Act
    let result = write_with_retry(&sink, &batch(&[1]), &policy).await;

    // Assert
    assert_eq!(result, Ok(()));
    assert_eq!(sink.call_count(), 2);
    assert_eq!(sink.len(), 1);
  }

  #[tokio::test(start_paused = true)]
  async fn backoff_grows_exponentially_and_is_capped() {
    // Arrange
    let sink = MemorySink::new();
    sink.fail_always(FailKind::Transient);
    let policy = RetryPolicy::new(
      4,
      Duration::from_millis(100),
      Duration::from_millis(250),
      2,
    )
    .expect("valid policy");
    let start = tokio::time::Instant::now();

    // Act
    let result = write_with_retry(&sink, &batch(&[1]), &policy).await;

    // Assert
    assert!(result.is_err());
    // 100 ms + 200 ms + 250 ms (capped); the fourth attempt is not delayed.
    assert_eq!(start.elapsed(), Duration::from_millis(550));
  }

  #[test]
  fn new_sink_uses_the_default_capacity() {
    // Arrange

    // Act
    let sink = MemorySink::new();

    // Assert
    assert_eq!(sink.capacity, Some(super::DEFAULT_MEMORY_SINK_CAPACITY));
  }

  #[test]
  fn memory_sink_name_is_stable() {
    // Arrange
    let sink = MemorySink::new();

    // Act
    let name = TraceSink::name(&sink);

    // Assert
    assert_eq!(name, "memory");
  }

  #[tokio::test]
  async fn shared_sink_handle_is_usable_through_a_trait_object() {
    // Arrange
    let sink = Arc::new(MemorySink::new());
    let shared = Arc::clone(&sink);
    let dyn_sink: Arc<dyn TraceSink> = shared;

    // Act
    dyn_sink
      .write_batch(&batch(&[9]))
      .await
      .expect("write succeeds");

    // Assert
    assert_eq!(sink.len(), 1);
  }
}
