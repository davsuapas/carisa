//! Tests of the public telemetry surface.
//!
//! This is an external crate, so it can only use what an application uses:
//! the configuration passed to
//! [`carisa_core::telemetry::Telemetry::builder`], the sinks and the error
//! types. Emitting records and driving the runtime lifecycle is carisa's job,
//! and the tests that exercise it live inside the module.

use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use carisa_core::{
  MemorySink, RetryPolicy, SinkError, Telemetry, TelemetryError, TraceRecord,
  TraceSink,
};
use rstest::rstest;

/// Sink implemented the way an application would implement one.
#[derive(Debug, Default)]
struct RecordingSink {
  batches: Mutex<Vec<usize>>,
}

#[async_trait::async_trait]
impl TraceSink for RecordingSink {
  fn name(&self) -> &'static str {
    "recording"
  }

  async fn write_batch(&self, batch: &[TraceRecord]) -> Result<(), SinkError> {
    self
      .batches
      .lock()
      .unwrap_or_else(std::sync::PoisonError::into_inner)
      .push(batch.len());

    Ok(())
  }
}

#[test]
fn a_telemetry_cannot_be_built_outside_a_runtime() {
  // Act
  let result = Telemetry::in_memory();

  // Assert
  assert_eq!(result.err(), Some(TelemetryError::NoRuntime));
}

#[tokio::test]
async fn a_telemetry_without_sinks_is_rejected() {
  // Act
  let result = Telemetry::builder().build();

  // Assert
  assert!(matches!(result, Err(TelemetryError::InvalidConfig(_))));
}

#[rstest]
#[case::zero_channel_capacity(0, 128, Duration::from_millis(1))]
#[case::zero_batch_size(1024, 0, Duration::from_millis(1))]
#[case::zero_flush_interval(1024, 128, Duration::ZERO)]
#[tokio::test]
async fn invalid_values_are_rejected(
  #[case] channel_capacity: usize,
  #[case] batch_size: usize,
  #[case] flush_interval: Duration,
) {
  // Act
  let result = Telemetry::builder()
    .sink(MemorySink::unbounded())
    .channel_capacity(channel_capacity)
    .batch_size(batch_size)
    .flush_interval(flush_interval)
    .build();

  // Assert
  assert!(matches!(result, Err(TelemetryError::InvalidConfig(_))));
}

#[tokio::test]
async fn the_default_configuration_is_accepted() {
  // Act
  let result = Telemetry::builder().sink(MemorySink::unbounded()).build();

  // Assert
  assert!(result.is_ok());
}

#[tokio::test]
async fn in_memory_builds_a_telemetry_and_an_empty_sink() {
  // Act
  let (telemetry, sink) = Telemetry::in_memory().expect("in memory");

  // Assert
  assert!(sink.is_empty());
  assert_eq!(sink.len(), 0);
  assert_eq!(sink.records().len(), 0);
  drop(telemetry);
}

#[tokio::test]
async fn a_sink_is_accepted_by_value_and_by_shared_pointer() {
  // Arrange
  let by_value = RecordingSink::default();
  let by_pointer = Arc::new(RecordingSink::default());

  // Act
  let result = Telemetry::builder()
    .sink(by_value)
    .sink_arc(by_pointer)
    .build();

  // Assert
  assert!(result.is_ok());
}

#[rstest]
#[case::zero_attempts(0, Duration::from_millis(1), Duration::from_secs(1), 2)]
#[case::zero_multiplier(3, Duration::from_millis(1), Duration::from_secs(1), 0)]
#[case::max_below_initial(3, Duration::from_secs(2), Duration::from_secs(1), 2)]
fn an_invalid_retry_policy_is_rejected(
  #[case] max_attempts: u32,
  #[case] initial_backoff: Duration,
  #[case] max_backoff: Duration,
  #[case] multiplier: u32,
) {
  // Act
  let result =
    RetryPolicy::new(max_attempts, initial_backoff, max_backoff, multiplier);

  // Assert
  assert!(matches!(result, Err(TelemetryError::InvalidConfig(_))));
}

#[test]
fn a_valid_retry_policy_exposes_its_values() {
  // Act
  let policy =
    RetryPolicy::new(4, Duration::from_millis(10), Duration::from_secs(1), 3)
      .expect("valid policy");

  // Assert
  assert_eq!(policy.max_attempts(), 4);
  assert_eq!(policy.initial_backoff(), Duration::from_millis(10));
  assert_eq!(policy.max_backoff(), Duration::from_secs(1));
  assert_eq!(policy.multiplier(), 3);
}

#[test]
fn the_default_retry_policy_is_valid() {
  // Act
  let policy = RetryPolicy::default();

  // Assert
  assert!(policy.max_attempts() > 0);
  assert!(policy.multiplier() > 0);
  assert!(policy.max_backoff() >= policy.initial_backoff());
}

#[test]
fn telemetry_errors_describe_the_failure() {
  // Arrange
  let cases = [
    (
      TelemetryError::InvalidConfig("bad value"),
      "invalid telemetry configuration: bad value",
    ),
    (
      TelemetryError::NoRuntime,
      "telemetry requires an active tokio runtime",
    ),
    (
      TelemetryError::ChannelClosed,
      "the telemetry worker is not running",
    ),
    (
      TelemetryError::WiringCheckFailed("not wired"),
      "telemetry wiring check failed: not wired",
    ),
  ];

  // Act & Assert
  for (error, expected) in cases {
    assert_eq!(error.to_string(), expected);
  }
}

#[test]
fn sink_errors_describe_the_failure() {
  // Arrange
  let cases = [
    (
      SinkError::Transient("timeout".to_owned()),
      "transient sink error: timeout",
    ),
    (
      SinkError::Permanent("bad schema".to_owned()),
      "permanent sink error: bad schema",
    ),
  ];

  // Act & Assert
  for (error, expected) in cases {
    assert_eq!(error.to_string(), expected);
  }
}

#[test]
fn the_crate_root_reexports_the_whole_configuration_surface() {
  // Arrange: naming every item from the crate root is the check itself.
  use carisa_core::{FieldValue, TelemetryBuilder, TraceLevel};

  // Act
  let level = TraceLevel::Info;
  let value = FieldValue::from(42_u64);
  let builder = TelemetryBuilder::default();

  // Assert
  assert!(matches!(level, TraceLevel::Info));
  assert_eq!(value, FieldValue::U64(42));
  drop(builder);
}
