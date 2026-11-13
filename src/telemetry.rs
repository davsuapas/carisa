//! Sink configuration for carisa core telemetry.
//!
//! Carisa marks part of its `tracing` events for persistence. Every marked
//! event becomes a [`TraceRecord`], and every record is written to the sinks
//! registered here: this module is the surface an application configures,
//! the destination of the records.
//!
//! The other half of telemetry is crate-internal. Which events are marked,
//! how they are tagged with the agent and the session identifiers, and the
//! lifecycle that flushes and closes the pipeline all belong to carisa: an
//! application neither emits a record nor closes telemetry itself. Inside the
//! crate those names are the `persist_event!`, `persist_info!`,
//! `persist_warn!`, `persist_error!` and `agent_span!` macros, plus the
//! crate-internal methods of [`Telemetry`]; all of it is documented in the
//! source, in the headers of the `layer` and `runtime` submodules.
//!
//! ```text
//!         (carisa)                      (this module)
//! marked event -> layer -> channel -> worker -+-> sink A
//!                                             +-> sink B
//!                                             +-> ...
//! ```
//!
//! # What an application configures
//!
//! Everything goes through [`Telemetry::builder`], and the resulting
//! [`Telemetry`] is handed to carisa:
//!
//! | Setting | Builder method | Default |
//! | --- | --- | --- |
//! | Sinks | `sink`, `sink_arc` | none: at least one required |
//! | Channel capacity | `channel_capacity` | 8 192 records |
//! | Batch size | `batch_size` | 256 records |
//! | Flush interval | `flush_interval` | 1 s |
//! | Retry policy | `retry` | 3 attempts, 100 ms to 5 s, x2 |
//!
//! `channel_capacity` and `batch_size` must be greater than zero and
//! `flush_interval` must not be zero; a [`RetryPolicy`] validates itself
//! when it is constructed. Any invalid value, and an empty sink list, make
//! [`TelemetryBuilder::build`] return [`TelemetryError::InvalidConfig`].
//!
//! Every registered sink receives every record: the worker fans a batch out
//! to all of them at the same time, so a failing sink never prevents the
//! others from receiving their copy.
//!
//! # Choosing a sink
//!
//! - [`MemorySink`] keeps records in memory. [`MemorySink::new`] bounds the
//!   store at 10 000 records and discards the oldest ones beyond that,
//!   [`MemorySink::unbounded`] keeps everything, and
//!   [`MemorySink::records`] reads them back. It is the sink to use in tests
//!   and in short-lived runs.
//! - A sink of your own: implement [`TraceSink`]. It is the extension point
//!   for a database, a file or a remote collector; see
//!   [Writing a sink](#writing-a-sink).
//!
//! # Quick start
//!
//! Every type below is also re-exported at the crate root, so the shortest
//! import is `use carisa_core::Telemetry;`. The examples use that form.
//!
//! The shortest valid configuration:
//!
//! ```rust,no_run
//! use carisa_core::Telemetry;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Building spawns the worker task, so it needs a tokio runtime.
//! let (telemetry, sink) = Telemetry::in_memory()?;
//!
//! // Hand `telemetry` to carisa. `sink` is the store the records land in,
//! // so a test can assert on them.
//! # drop(telemetry);
//! # Ok(())
//! # }
//! ```
//!
//! Every setting at once, with one sink inline and one already shared:
//!
//! ```rust,no_run
//! # use std::sync::Arc;
//! # use std::time::Duration;
//! use carisa_core::{
//!   MemorySink, RetryPolicy, SinkError, Telemetry, TraceRecord, TraceSink,
//! };
//! # use async_trait::async_trait;
//! # #[derive(Debug)]
//! # struct MySink;
//! # #[async_trait]
//! # impl TraceSink for MySink {
//! #   fn name(&self) -> &str { "my-sink" }
//! #
//! #   async fn write_batch(&self, _: &[TraceRecord])
//! #   -> Result<(), SinkError> {
//! #     Ok(())
//! #   }
//! # }
//! # fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let shared: Arc<dyn TraceSink> = Arc::new(MySink);
//!
//! let telemetry = Telemetry::builder()
//!   .sink(MemorySink::with_capacity(1_000))
//!   .sink_arc(shared)
//!   .channel_capacity(16_384)
//!   .batch_size(512)
//!   .flush_interval(Duration::from_millis(500))
//!   .retry(RetryPolicy::new(
//!     5,
//!     Duration::from_millis(50),
//!     Duration::from_secs(2),
//!     2,
//!   )?)
//!   .build()?;
//! # drop(telemetry);
//! # Ok(())
//! # }
//! ```
//!
//! # What a sink receives
//!
//! A [`TraceRecord`] is a plain owned value, already resolved, with
//! everything a sink needs to store the event:
//!
//! - `seq`: process-wide increasing number assigned before the record is
//!   queued. A gap in the sequence means a record was dropped before it
//!   reached a sink. It is the only ordering key a sink can trust:
//!   `timestamp_ms` uses the wall clock and can move backwards.
//! - `level`, `target` and `message`: the severity, the module path that
//!   produced the event, and the formatted message.
//! - `fields`: the extra key/value pairs of the event. [`FieldValue`] lists
//!   the kinds that can be stored.
//! - `agent_id` and `session_id`: the identifiers of the agent span that
//!   surrounds the event, or `None` when it was emitted outside such a span.
//!   [`TraceRecord::agent_id`] and [`TraceRecord::session_id`] return them
//!   as `Option<&str>`.
//!
//! # Writing a sink
//!
//! ```rust
//! use async_trait::async_trait;
//! use carisa_core::{SinkError, TraceRecord, TraceSink};
//!
//! #[derive(Debug)]
//! struct StdoutSink;
//!
//! #[async_trait]
//! impl TraceSink for StdoutSink {
//!   fn name(&self) -> &'static str {
//!     "stdout"
//!   }
//!
//!   async fn write_batch(&self, batch: &[TraceRecord])
//!   -> Result<(), SinkError> {
//!     for record in batch {
//!       // Retries can resend a batch after a partial write: a backend
//!       // that is not idempotent must deduplicate by `seq`.
//!       println!("{} {:?} {}", record.seq, record.agent_id(), record.message);
//!     }
//!     Ok(())
//!   }
//! }
//! ```
//!
//! Return [`SinkError::Transient`] for timeouts, connection resets and rate
//! limits: the whole batch is retried. Return [`SinkError::Permanent`] for
//! authentication or schema errors: the batch is abandoned. Keep
//! [`TraceSink::write_batch`] fast, because a slow sink delays every other
//! sink.
//!
//! # Errors
//!
//! [`TelemetryError`] is the only error an application sees:
//!
//! | Variant | Raised when |
//! | --- | --- |
//! | `InvalidConfig` | a value is out of range or no sink was registered |
//! | `NoRuntime` | telemetry was built without a tokio runtime |
//! | `ChannelClosed` | the worker is not running |
//! | `WiringCheckFailed` | carisa could not confirm the wiring |
//!
//! A failing sink never surfaces as an error to the application: it is
//! retried and accounted for inside carisa.
//!
//! # Guarantees a sink can rely on
//!
//! | Aspect | Guarantee |
//! | --- | --- |
//! | Blocking | An event never blocks the thread that emitted it. |
//! | Record loss | Possible: a full or closed channel counts a drop. |
//! | Delivery | At-least-once per sink; retries may duplicate. |
//! | Ordering | Per emitting thread; sort by `seq` for a global order. |
//! | A gap in `seq` | A record was dropped on the way to the sinks. |
//! | `timestamp_ms` | Wall clock; it can move backwards. |
//! | Sink isolation | A failed or panicking sink spares the others. |
//! | Sink speed | A slow sink delays all the others. |
//! | Panic in a sink | Captured and counted, unless `panic = "abort"`. |
//! | Sensitive data | No redaction in v1. |
//!
//! # Privacy
//!
//! Version 1 does **not** redact anything: every field carisa marks for
//! persistence is stored as is, in every sink. Which fields are marked is
//! carisa's decision, so a sink must treat the records it receives as
//! potentially sensitive data and protect its own storage.

#![expect(
  dead_code,
  unused_imports,
  unused_macros,
  reason = "the emission half of this module (persist_*! and agent_span!, the \
            layer and the runtime lifecycle) is consumed by carisa itself, \
            which does not use telemetry yet; every crate-internal item is \
            exercised by the tests of the module. Remove once carisa emits."
)]

use std::time::Duration;

mod layer;
mod record;
mod runtime;
mod sink;

#[cfg(test)]
mod pipeline;

// Public surface: how an application configures where the records go. It is
// mirrored at the crate root, so `use carisa_core::{MemorySink, Telemetry};`
// works too.
pub use record::{FieldValue, TraceLevel, TraceRecord};
pub use runtime::{Telemetry, TelemetryBuilder, TelemetryError};
pub use sink::{MemorySink, RetryPolicy, SinkError, TraceSink};

// Internal surface: how carisa emits records and drives the runtime. None of
// it is visible outside the crate. The emission macros are re-exported at the
// end of the file, after their definition.
pub(crate) use runtime::{FlushReport, TelemetryStats};
#[cfg(test)]
pub(crate) use sink::FailKind;

/// Reserved `tracing` target of the events that must be persisted.
///
/// Crate-internal: only the `persist_*!` macros of this crate emit it.
pub(crate) const PERSIST_TARGET: &str = "telemetry::persist";

/// Reserved `tracing` target of the module's own diagnostics.
pub(crate) const INTERNAL_TARGET: &str = "telemetry::internal";

/// Field used by the `check_wiring` probes.
pub(crate) const PROBE_FIELD: &str = "telemetry_probe";

/// Field that declares the agent identifier of a span.
pub(crate) const AGENT_ID_FIELD: &str = "agent_id";

/// Field that declares the session identifier of a span.
pub(crate) const SESSION_ID_FIELD: &str = "session_id";

/// Field that carries the message of an event.
pub(crate) const MESSAGE_FIELD: &str = "message";

/// Default capacity of the `MemorySink` store.
pub(crate) const DEFAULT_MEMORY_SINK_CAPACITY: usize = 10_000;

/// Default capacity of the channel between the layer and the worker.
pub(crate) const DEFAULT_CHANNEL_CAPACITY: usize = 8192;

/// Default number of records the worker accumulates before dispatching.
pub(crate) const DEFAULT_BATCH_SIZE: usize = 256;

/// Default time the worker waits before dispatching an incomplete batch.
pub(crate) const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_secs(1);

/// `tracing` re-export used by the macros, so they do not depend on the
/// caller having `tracing` in scope under that name.
pub(crate) use ::tracing as __tracing;

/// Persists a `tracing` event with an explicit level.
///
/// Use it for levels without a dedicated shortcut (`DEBUG`, `TRACE`). There
/// is no level threshold: every event written through this macro is
/// persisted, so consider the volume before using a low level.
///
/// Crate-internal: reachable as `crate::telemetry::persist_event!`.
macro_rules! persist_event {
  ($level:expr, $($arg:tt)*) => {
    $crate::telemetry::__tracing::event!(
      target: $crate::telemetry::PERSIST_TARGET,
      $level,
      $($arg)*
    )
  };
}

/// Persists an `INFO` event.
///
/// Crate-internal: reachable as `crate::telemetry::persist_info!`.
macro_rules! persist_info {
  ($($arg:tt)*) => {
    $crate::telemetry::__tracing::info!(
      target: $crate::telemetry::PERSIST_TARGET,
      $($arg)*
    )
  };
}

/// Persists a `WARN` event.
///
/// Crate-internal: reachable as `crate::telemetry::persist_warn!`.
macro_rules! persist_warn {
  ($($arg:tt)*) => {
    $crate::telemetry::__tracing::warn!(
      target: $crate::telemetry::PERSIST_TARGET,
      $($arg)*
    )
  };
}

/// Persists an `ERROR` event.
///
/// Crate-internal: reachable as `crate::telemetry::persist_error!`.
macro_rules! persist_error {
  ($($arg:tt)*) => {
    $crate::telemetry::__tracing::error!(
      target: $crate::telemetry::PERSIST_TARGET,
      $($arg)*
    )
  };
}

/// Creates the `agent` span that tags records with both identifiers.
///
/// The two-argument form uses `INFO`; the three-argument form takes an
/// explicit level first. A trailing comma is accepted.
///
/// Crate-internal: reachable as `crate::telemetry::agent_span!`.
macro_rules! agent_span {
  ($agent_id:expr, $session_id:expr $(,)?) => {
    $crate::telemetry::__tracing::info_span!(
      "agent",
      agent_id = %$agent_id,
      session_id = %$session_id
    )
  };
  ($level:expr, $agent_id:expr, $session_id:expr $(,)?) => {
    $crate::telemetry::__tracing::span!(
      $level,
      "agent",
      agent_id = %$agent_id,
      session_id = %$session_id
    )
  };
}

// Internal surface (continued): the macros are re-exported here, after their
// definition, so that every module of carisa can import them by name or use
// them as `crate::telemetry::persist_info!`.
pub(crate) use {
  agent_span, persist_error, persist_event, persist_info, persist_warn,
};
