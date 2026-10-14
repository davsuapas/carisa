//! Immutable record types produced by the telemetry layer.
//!
//! A [`TraceRecord`] is the unit of work that travels from the `tracing`
//! layer to a [`crate::telemetry::TraceSink`]. It is deliberately a plain
//! owned value: the hot path builds it on the calling thread and hands it
//! over to the worker through a bounded channel, so no reference to the
//! `tracing` registry survives the call.
//!
//! Two details are worth keeping in mind when consuming records:
//!
//! - `seq` is assigned from a process-wide atomic counter *before* the
//!   record is sent. A gap in the sequence means a record was dropped
//!   because the channel was full or closed. Records are ordered by `seq`
//!   per emitting thread; across threads the order of arrival may differ
//!   slightly, so consumers that need a strict global order should sort by
//!   `seq`.
//! - `timestamp_ms` uses the wall clock, which can move backwards. Never
//!   use it for ordering; use `seq` instead.
//!
//! Field values are mapped from the `tracing` macro syntax as follows:
//!
//! | Written in the macro | Stored as |
//! | --- | --- |
//! | `&str`, `String` | `Str` |
//! | `i8`..`i64`, `isize` | `I64` |
//! | `u8`..`u64`, `usize` | `U64` |
//! | `f32`, `f64` | `F64` |
//! | `bool` | `Bool` |
//! | `i128`, `u128`, `?value`, `%value`, errors | `Str` |

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// Process-wide counter backing [`TraceRecord::seq`].
///
/// It is a plain `static` rather than part of the runtime state because the
/// sequence must keep growing even if several `Telemetry` instances coexist
/// in the same process; gaps are meaningful regardless of which instance
/// dropped the record.
static NEXT_SEQ: AtomicU64 = AtomicU64::new(0);

/// Returns the next value of the process-wide record sequence.
fn next_seq() -> u64 {
  NEXT_SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Returns the current wall-clock time in milliseconds since the Unix epoch.
///
/// Before the epoch, or if the clock is unsupported, it saturates to `0`
/// instead of panicking.
fn now_ms() -> u64 {
  let elapsed = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default();

  u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

/// Severity of a [`TraceRecord`].
///
/// The ordering follows `tracing`: `Error` is the lowest value and `Trace`
/// the highest. The level is informational in v1 — there is no level
/// threshold — so it never causes a record to be discarded.
#[expect(
  clippy::exhaustive_enums,
  reason = "the set of tracing levels is closed and consumers match on it"
)]
#[repr(u8)]
#[derive(
  Debug,
  Clone,
  Copy,
  PartialEq,
  Eq,
  PartialOrd,
  Ord,
  Hash,
  Serialize,
  Deserialize,
)]
pub enum TraceLevel {
  /// An error-level event.
  Error,
  /// A warning-level event.
  Warn,
  /// An informational event.
  Info,
  /// A debug-level event.
  Debug,
  /// A trace-level event.
  Trace,
}

impl TraceLevel {
  /// Returns the numeric representation of the level.
  ///
  /// The value is stable and matches the `u8` discriminant of the variant.
  #[must_use]
  pub const fn as_u8(self) -> u8 {
    self as u8
  }
}

impl From<&tracing::Level> for TraceLevel {
  fn from(level: &tracing::Level) -> Self {
    match *level {
      tracing::Level::ERROR => Self::Error,
      tracing::Level::WARN => Self::Warn,
      tracing::Level::INFO => Self::Info,
      tracing::Level::DEBUG => Self::Debug,
      tracing::Level::TRACE => Self::Trace,
    }
  }
}

/// A single field attached to a persisted event.
///
/// `i128`, `u128`, `?value` (Debug) and `%value` (Display) are converted to
/// [`FieldValue::Str`], so a value is never lost — only its native type.
#[expect(
  clippy::exhaustive_enums,
  reason = "the mapping from tracing values to this enum is closed"
)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FieldValue {
  /// A textual value.
  Str(String),
  /// A signed integer that fits in `i64`.
  I64(i64),
  /// An unsigned integer that fits in `u64`.
  U64(u64),
  /// A floating point value.
  F64(f64),
  /// A boolean value.
  Bool(bool),
}

impl From<bool> for FieldValue {
  fn from(value: bool) -> Self {
    Self::Bool(value)
  }
}

impl From<i64> for FieldValue {
  fn from(value: i64) -> Self {
    Self::I64(value)
  }
}

impl From<u64> for FieldValue {
  fn from(value: u64) -> Self {
    Self::U64(value)
  }
}

impl From<f64> for FieldValue {
  fn from(value: f64) -> Self {
    Self::F64(value)
  }
}

impl From<String> for FieldValue {
  fn from(value: String) -> Self {
    Self::Str(value)
  }
}

/// A persisted `tracing` event, tagged with agent and session identifiers.
///
/// The type is `#[non_exhaustive]`: sinks read its fields but the library is
/// the only producer. Use [`TraceRecord::agent_id`] and
/// [`TraceRecord::session_id`] to read the identifiers as `&str`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraceRecord {
  /// Process-wide sequence number, assigned before the record is enqueued.
  ///
  /// A gap means a record was dropped. Sort by this field to get a global
  /// order.
  pub seq: u64,
  /// Wall-clock timestamp in milliseconds since the Unix epoch.
  ///
  /// Not monotonic; never use it for ordering.
  pub timestamp_ms: u64,
  /// Severity of the originating event.
  pub level: TraceLevel,
  /// Module path of the caller that emitted the event.
  ///
  /// This is the real `module_path`, not the reserved target the event was
  /// emitted with.
  pub target: Cow<'static, str>,
  /// Message of the event, after formatting its arguments.
  pub message: String,
  /// Identifier of the agent the event belongs to, if any.
  pub agent_id: Option<Arc<str>>,
  /// Identifier of the session the event belongs to, if any.
  pub session_id: Option<Arc<str>>,
  /// Extra fields declared by the event, in declaration order.
  ///
  /// Empty (and not allocating) when the event has no extra fields.
  pub fields: Vec<(Cow<'static, str>, FieldValue)>,
}

impl TraceRecord {
  /// Builds a record without agent or session context.
  pub(crate) fn new(
    level: TraceLevel,
    target: Cow<'static, str>,
    message: String,
    fields: Vec<(Cow<'static, str>, FieldValue)>,
  ) -> Self {
    Self {
      seq: next_seq(),
      timestamp_ms: now_ms(),
      level,
      target,
      message,
      agent_id: None,
      session_id: None,
      fields,
    }
  }

  /// Attaches the resolved agent and session identifiers.
  ///
  /// Consumes and returns the record so the caller can chain the call
  /// without an extra binding.
  pub(crate) fn with_context(
    mut self,
    agent_id: Option<Arc<str>>,
    session_id: Option<Arc<str>>,
  ) -> Self {
    self.agent_id = agent_id;
    self.session_id = session_id;
    self
  }

  /// Returns the agent identifier as a string slice.
  #[must_use]
  pub fn agent_id(&self) -> Option<&str> {
    self.agent_id.as_deref()
  }

  /// Returns the session identifier as a string slice.
  #[must_use]
  pub fn session_id(&self) -> Option<&str> {
    self.session_id.as_deref()
  }
}

#[cfg(test)]
mod tests {
  use std::borrow::Cow;
  use std::sync::Arc;

  use rstest::rstest;

  use super::{FieldValue, TraceLevel, TraceRecord, next_seq};

  fn sample_record() -> TraceRecord {
    TraceRecord::new(
      TraceLevel::Info,
      Cow::Borrowed("carisa_core::worker"),
      "agent started".to_owned(),
      vec![
        (Cow::Borrowed("tool"), FieldValue::from("search".to_owned())),
        (Cow::Borrowed("tokens"), FieldValue::from(12_u64)),
      ],
    )
    .with_context(Some(Arc::from("agent-1")), Some(Arc::from("sess-1")))
  }

  #[test]
  fn full_record_survives_a_serde_round_trip() {
    // Arrange
    let record = sample_record();

    // Act
    let json = serde_json::to_string(&record).expect("record serializes");
    let restored: TraceRecord =
      serde_json::from_str(&json).expect("record deserializes");

    // Assert
    assert_eq!(restored, record);
    assert_eq!(restored.agent_id(), Some("agent-1"));
    assert_eq!(restored.session_id(), Some("sess-1"));
  }

  #[test]
  fn record_without_context_survives_a_serde_round_trip() {
    // Arrange
    let record = TraceRecord::new(
      TraceLevel::Warn,
      Cow::Borrowed("carisa_core::worker"),
      "no context".to_owned(),
      Vec::new(),
    );

    // Act
    let json = serde_json::to_string(&record).expect("record serializes");
    let restored: TraceRecord =
      serde_json::from_str(&json).expect("record deserializes");

    // Assert
    assert_eq!(restored, record);
    assert_eq!(restored.agent_id(), None);
    assert_eq!(restored.session_id(), None);
    assert_eq!(restored.fields.len(), 0);
  }

  #[rstest]
  #[case(tracing::Level::ERROR, TraceLevel::Error)]
  #[case(tracing::Level::WARN, TraceLevel::Warn)]
  #[case(tracing::Level::INFO, TraceLevel::Info)]
  #[case(tracing::Level::DEBUG, TraceLevel::Debug)]
  #[case(tracing::Level::TRACE, TraceLevel::Trace)]
  fn tracing_level_maps_to_the_matching_variant(
    #[case] level: tracing::Level,
    #[case] expected: TraceLevel,
  ) {
    // Arrange

    // Act
    let mapped = TraceLevel::from(&level);

    // Assert
    assert_eq!(mapped, expected);
  }

  #[test]
  fn seq_is_strictly_increasing_for_one_thread() {
    // Arrange
    let first = next_seq();
    let second = next_seq();
    let third = next_seq();

    // Act

    // Assert
    assert!(first < second);
    assert!(second < third);
  }

  #[test]
  fn level_as_u8_matches_the_declared_discriminant() {
    // Arrange
    let cases = [
      (TraceLevel::Error, 0),
      (TraceLevel::Warn, 1),
      (TraceLevel::Info, 2),
      (TraceLevel::Debug, 3),
      (TraceLevel::Trace, 4),
    ];

    // Act

    // Assert
    for (level, expected) in cases {
      assert_eq!(level.as_u8(), expected);
    }
  }

  #[test]
  fn field_value_conversions_keep_the_native_kind() {
    // Arrange
    let cases = [
      (FieldValue::from(true), FieldValue::Bool(true)),
      (FieldValue::from(-1_i64), FieldValue::I64(-1)),
      (FieldValue::from(7_u64), FieldValue::U64(7)),
      (FieldValue::from(1.5_f64), FieldValue::F64(1.5)),
      (
        FieldValue::from("text".to_owned()),
        FieldValue::Str("text".to_owned()),
      ),
    ];

    // Act

    // Assert
    for (actual, expected) in cases {
      assert_eq!(actual, expected);
    }
  }
}
