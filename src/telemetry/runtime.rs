//! Runtime: builder, handle, worker and statistics.
//!
//! A [`Telemetry`] owns a bounded channel and the worker task that drains it.
//! The layer enqueues records with `try_send`, so an event never blocks the
//! caller: if the channel is full or closed the record is counted as dropped
//! and the application keeps running.
//!
//! The worker accumulates records in a reusable buffer and dispatches them
//! when the buffer reaches the batch size or when the flush interval expires.
//! Data and control messages share the same channel so ordering is preserved:
//! everything enqueued before a `flush` or a `shutdown` is attempted before
//! the report is produced.
//!
//! Dispatching a batch fans it out to every sink at the same time. A sink
//! that fails, or even panics, does not prevent the others from receiving the
//! data, but it does delay them: the worker processes one batch at a time.
//!
//! # Lifecycle
//!
//! Crate-internal. Carisa composes the subscriber once, keeps the handle
//! alive and closes it before the process ends:
//!
//! ```text
//! let (telemetry, _sink) = Telemetry::in_memory()?;   // needs a runtime
//! registry().with(fmt_layer).with(telemetry.layer()).try_init()?;
//! telemetry.check_wiring()?;                          // fail fast
//! ... carisa emits inside agent_span!(..) ...
//! let report = telemetry.shutdown().await?;           // clean close
//! ```
//!
//! - One `Telemetry` per process: `layer()` can be consumed once, and a
//!   second layer is inert instead of duplicating records.
//! - `check_wiring()` after the subscriber is installed. It catches the
//!   failures that would otherwise be silent: a global `EnvFilter` that
//!   disables the callsite for every layer, the `max_level_*` features of
//!   `tracing`, a layer that was never composed, and a `try_init` that
//!   returned `Err` because another global subscriber already existed.
//! - `flush().await` is a barrier: everything enqueued before the call is
//!   attempted before the report is produced. `shutdown().await` consumes
//!   the handle and is the only clean close; `Drop` is best effort and warns
//!   when records were still pending.
//! - Call `shutdown()` before the runtime ends: dropping the runtime stops
//!   the worker. Waiting for `Ctrl-C` or `SIGTERM` is the application's job,
//!   not the module's.
//! - `stats()` reports what the pipeline accepted, dropped and lost;
//!   `missing_context` counts records stored without an identifier, which is
//!   a propagation mistake, not a loss.
//!
//! # Verification
//!
//! The behaviour above is covered by the tests of this file, among them
//! `worker_dispatches_when_the_batch_is_full`,
//! `worker_dispatches_an_incomplete_batch_when_the_interval_expires`,
//! `flush_preserves_the_order_of_the_records`,
//! `shutdown_delivers_everything_pending`,
//! `fan_out_sends_every_record_to_every_sink`,
//! `a_failing_sink_does_not_stop_the_others`,
//! `a_batch_that_recovers_after_retries_is_delivered`,
//! `a_panicking_sink_is_contained`,
//! `check_wiring_fails_with_a_global_filter_that_drops_info`,
//! `check_wiring_fails_when_the_layer_is_not_composed`,
//! `dropping_with_pending_records_warns_and_does_not_panic`,
//! `events_after_shutdown_are_dropped` and
//! `stats_report_what_the_layer_accepted_and_dropped`. The whole pipeline is
//! exercised end to end by the tests of `pipeline.rs`.

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use derive_builder::Builder;
use futures_util::FutureExt;
use futures_util::future::join_all;
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};
use tracing::{Level, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::registry::LookupSpan;

use super::layer::{PersistLayer, is_persisted};
use super::record::TraceRecord;
use super::sink::{MemorySink, RetryPolicy, TraceSink, write_with_retry};
use super::{
  DEFAULT_BATCH_SIZE, DEFAULT_CHANNEL_CAPACITY, DEFAULT_FLUSH_INTERVAL,
  INTERNAL_TARGET, PERSIST_TARGET,
};

/// Errors reported by the telemetry runtime.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TelemetryError {
  /// The builder configuration is invalid.
  #[error("invalid telemetry configuration: {0}")]
  InvalidConfig(&'static str),
  /// There is no tokio runtime where the telemetry was built.
  #[error("telemetry requires an active tokio runtime")]
  NoRuntime,
  /// The worker is no longer running, so the channel is closed.
  #[error("the telemetry worker is not running")]
  ChannelClosed,
  /// The wiring check proved that the layer does not receive events.
  #[error("telemetry wiring check failed: {0}")]
  WiringCheckFailed(&'static str),
}

/// Outcome of a [`Telemetry::flush`] or [`Telemetry::shutdown`].
///
/// Counts are per (record, sink) pair and cover the records processed since
/// the previous report, so a report with no new data is all zeros.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(
  clippy::redundant_pub_crate,
  reason = "re-exported crate-wide by the module root"
)]
pub(crate) struct FlushReport {
  /// Pairs written successfully by their sink.
  pub delivered: u64,
  /// Pairs whose sink failed after exhausting the retries.
  pub lost: u64,
}

impl FlushReport {
  /// Returns `true` when nothing was lost.
  #[must_use]
  pub(crate) const fn is_complete(&self) -> bool {
    self.lost == 0
  }
}

/// Snapshot of the telemetry counters.
///
/// `delivered` and `lost_records` count (record, sink) pairs, not records.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(
  clippy::redundant_pub_crate,
  reason = "re-exported crate-wide by the module root"
)]
pub(crate) struct TelemetryStats {
  /// Records accepted by the channel.
  pub enqueued: u64,
  /// Records rejected because the channel was full or closed.
  pub dropped: u64,
  /// Records enqueued without an agent or session identifier.
  ///
  /// They were stored, not dropped.
  pub missing_context: u64,
  /// Pairs written successfully by their sink.
  pub delivered: u64,
  /// Batches abandoned by a sink after exhausting the retries.
  pub failed_batches: u64,
  /// Pairs reported as lost.
  pub lost_records: u64,
}

/// Counters and flags shared by the layer, the worker and the handle.
///
/// Every counter uses `Ordering::Relaxed`: they are statistics, not
/// synchronization primitives, so no ordering between them is meaningful.
#[derive(Debug, Default)]
pub(super) struct Shared {
  enqueued: AtomicU64,
  dropped: AtomicU64,
  missing_context: AtomicU64,
  delivered: AtomicU64,
  failed_batches: AtomicU64,
  lost_records: AtomicU64,
  processed: AtomicU64,
  probe_event_seen: AtomicBool,
  probe_span_seen: AtomicBool,
  layer_taken: AtomicBool,
  closed: AtomicBool,
}

impl Shared {
  /// Creates the shared state with every counter at zero.
  pub(super) fn new() -> Self {
    Self::default()
  }

  /// Counts a record accepted by the channel.
  pub(super) fn record_enqueued(&self) {
    self.enqueued.fetch_add(1, Ordering::Relaxed);
  }

  /// Counts a record rejected by the channel.
  pub(super) fn record_dropped(&self) {
    self.dropped.fetch_add(1, Ordering::Relaxed);
  }

  /// Counts a record enqueued without a complete context.
  pub(super) fn record_missing_context(&self) {
    self.missing_context.fetch_add(1, Ordering::Relaxed);
  }

  /// Flags that the wiring probe event reached the layer.
  pub(super) fn mark_probe_event(&self) {
    self.probe_event_seen.store(true, Ordering::Relaxed);
  }

  /// Flags that the wiring probe span reached the layer.
  pub(super) fn mark_probe_span(&self) {
    self.probe_span_seen.store(true, Ordering::Relaxed);
  }

  /// Reserves the layer slot, returning `true` when it was already taken.
  pub(super) fn take_layer(&self) -> bool {
    self.layer_taken.swap(true, Ordering::Relaxed)
  }

  /// Clears the wiring probe flags.
  fn reset_probes(&self) {
    self.probe_event_seen.store(false, Ordering::Relaxed);
    self.probe_span_seen.store(false, Ordering::Relaxed);
  }

  /// Returns `true` when the wiring probe event was seen.
  fn probe_event_seen(&self) -> bool {
    self.probe_event_seen.load(Ordering::Relaxed)
  }

  /// Returns `true` when the wiring probe span was seen.
  fn probe_span_seen(&self) -> bool {
    self.probe_span_seen.load(Ordering::Relaxed)
  }

  /// Marks the handle as closed.
  fn close(&self) {
    self.closed.store(true, Ordering::Relaxed);
  }

  /// Returns `true` when the handle was already consumed by a shutdown.
  pub(super) fn is_closed(&self) -> bool {
    self.closed.load(Ordering::Relaxed)
  }

  /// Counts records accepted by the channel.
  fn enqueued(&self) -> u64 {
    self.enqueued.load(Ordering::Relaxed)
  }

  /// Counts records handed over to the sinks.
  fn processed(&self) -> u64 {
    self.processed.load(Ordering::Relaxed)
  }

  /// Adds `count` delivered pairs.
  fn add_delivered(&self, count: u64) {
    self.delivered.fetch_add(count, Ordering::Relaxed);
  }

  /// Adds `count` lost pairs.
  fn add_lost(&self, count: u64) {
    self.lost_records.fetch_add(count, Ordering::Relaxed);
  }

  /// Counts a batch abandoned by a sink.
  fn add_failed_batch(&self) {
    self.failed_batches.fetch_add(1, Ordering::Relaxed);
  }

  /// Adds `count` records handed over to the sinks.
  fn add_processed(&self, count: u64) {
    self.processed.fetch_add(count, Ordering::Relaxed);
  }

  /// Reads the delivered counter.
  fn delivered(&self) -> u64 {
    self.delivered.load(Ordering::Relaxed)
  }

  /// Reads the lost counter.
  fn lost_records(&self) -> u64 {
    self.lost_records.load(Ordering::Relaxed)
  }

  /// Takes a snapshot of every counter.
  fn stats(&self) -> TelemetryStats {
    TelemetryStats {
      enqueued: self.enqueued(),
      dropped: self.dropped.load(Ordering::Relaxed),
      missing_context: self.missing_context.load(Ordering::Relaxed),
      delivered: self.delivered(),
      failed_batches: self.failed_batches.load(Ordering::Relaxed),
      lost_records: self.lost_records(),
    }
  }
}

/// Message exchanged with the worker.
///
/// Control messages travel through the same channel as the records so that
/// everything enqueued before them is dispatched first.
#[derive(Debug)]
pub(super) enum Msg {
  /// A record to persist.
  Record(TraceRecord),
  /// Dispatch what is buffered and report the counters.
  Flush(oneshot::Sender<FlushReport>),
  /// Dispatch what is buffered, report and stop the worker.
  Shutdown(oneshot::Sender<FlushReport>),
}

/// Configuration produced by [`TelemetryBuilder`].
///
/// The type stays private: the public builder is the only way to create it,
/// so no invalid `TelemetryConfig` can exist.
#[derive(Builder)]
#[builder(
  name = "TelemetryBuilder",
  pattern = "owned",
  vis = "pub",
  build_fn(
    private,
    name = "build_config",
    validate = "Self::validate",
    error = "TelemetryError"
  )
)]
struct TelemetryConfig {
  /// Sinks that receive every record; at least one is required.
  #[builder(default, setter(custom))]
  sinks: Vec<Arc<dyn TraceSink>>,
  /// Capacity of the channel between the layer and the worker.
  #[builder(default = "DEFAULT_CHANNEL_CAPACITY")]
  channel_capacity: usize,
  /// Records accumulated before the worker dispatches a batch.
  #[builder(default = "DEFAULT_BATCH_SIZE")]
  batch_size: usize,
  /// Time the worker waits before dispatching an incomplete batch.
  #[builder(default = "DEFAULT_FLUSH_INTERVAL")]
  flush_interval: Duration,
  /// Retry policy applied to every sink.
  #[builder(default)]
  retry: RetryPolicy,
}

impl TelemetryBuilder {
  /// Rejects the values that cannot produce a working runtime.
  fn validate(builder: &Self) -> Result<(), TelemetryError> {
    if builder.sinks.as_ref().is_none_or(Vec::is_empty) {
      return Err(TelemetryError::InvalidConfig(
        "at least one sink is required",
      ));
    }

    if builder.channel_capacity == Some(0) {
      return Err(TelemetryError::InvalidConfig(
        "channel_capacity must be greater than zero",
      ));
    }

    if builder.batch_size == Some(0) {
      return Err(TelemetryError::InvalidConfig(
        "batch_size must be greater than zero",
      ));
    }

    if builder.flush_interval == Some(Duration::ZERO) {
      return Err(TelemetryError::InvalidConfig(
        "flush_interval must be greater than zero",
      ));
    }

    Ok(())
  }

  /// Registers a sink, wrapping it in an `Arc` internally.
  ///
  /// Can be called several times; every registered sink receives every
  /// record.
  #[must_use]
  pub fn sink(mut self, sink: impl TraceSink + 'static) -> Self {
    self.sinks.get_or_insert_with(Vec::new).push(Arc::new(sink));
    self
  }

  /// Registers a sink that is already shared.
  #[must_use]
  pub fn sink_arc(mut self, sink: Arc<dyn TraceSink>) -> Self {
    self.sinks.get_or_insert_with(Vec::new).push(sink);
    self
  }

  /// Builds the runtime and starts the worker.
  ///
  /// # Errors
  ///
  /// Returns [`TelemetryError::InvalidConfig`] when the configuration is
  /// invalid, and [`TelemetryError::NoRuntime`] when it is called outside a
  /// tokio runtime.
  pub fn build(self) -> Result<Telemetry, TelemetryError> {
    let config = self.build_config()?;

    if tokio::runtime::Handle::try_current().is_err() {
      return Err(TelemetryError::NoRuntime);
    }

    Ok(Telemetry::spawn(config))
  }
}

/// Handle to the telemetry runtime.
///
/// The handle is created with [`Telemetry::builder`] or
/// [`Telemetry::in_memory`] and handed to carisa, which owns it: only carisa
/// composes the `tracing` layer and closes the pipeline before the process
/// ends. Dropping the handle stops persistence; the drop implementation only
/// sends a best-effort stop request and never waits.
#[must_use = "dropping Telemetry stops persistence; keep it until shutdown"]
#[derive(Debug)]
pub struct Telemetry {
  sender: mpsc::Sender<Msg>,
  shared: Arc<Shared>,
}

impl Telemetry {
  /// Returns a builder with the documented defaults.
  #[must_use]
  pub fn builder() -> TelemetryBuilder {
    TelemetryBuilder::default()
  }

  /// Builds a runtime backed by a bounded [`MemorySink`].
  ///
  /// Returns the handle and a clone of the sink, so tests can inspect what
  /// was persisted. Intended for development and tests.
  ///
  /// # Errors
  ///
  /// Returns the same errors as [`TelemetryBuilder::build`].
  pub fn in_memory() -> Result<(Self, MemorySink), TelemetryError> {
    let sink = MemorySink::new();
    let handle = sink.clone();
    let telemetry = Self::builder().sink(sink).build()?;

    Ok((telemetry, handle))
  }

  /// Returns the layer to compose into the subscriber.
  ///
  /// The layer can be obtained only once: a second call returns an inert
  /// layer and logs a warning on the target reserved for the module, so
  /// composing it twice cannot duplicate records.
  ///
  /// The returned layer is filtered per layer, which means it does not affect
  /// the callsites of any other layer.
  pub(crate) fn layer<S>(&self) -> impl Layer<S> + Send + Sync + 'static
  where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
  {
    if self.shared.take_layer() {
      tracing::warn!(
        target: INTERNAL_TARGET,
        "the telemetry layer was requested more than once; \
         returning an inert layer"
      );
      return None;
    }

    Some(
      PersistLayer::new(self.sender.clone(), Arc::clone(&self.shared))
        .with_filter(filter_fn(is_persisted)),
    )
  }

  /// Checks that events and spans reach the layer.
  ///
  /// Call it after `try_init()` (or after installing the subscriber as the
  /// default of the thread). It emits a probe span and a probe event through
  /// the dispatcher that is active on the calling thread and reports whether
  /// they arrived. The probes are never enqueued and never counted in the
  /// statistics.
  ///
  /// # Errors
  ///
  /// Returns [`TelemetryError::WiringCheckFailed`] when the probe span or the
  /// probe event does not reach the layer. The message lists the likely
  /// causes: a global filter (for example a plain `EnvFilter` layer) that
  /// drops `INFO`, a `tracing` `max_level_*` feature that compiles the level
  /// out, a layer that was never composed, or a `try_init()` that failed
  /// because another global subscriber already exists.
  pub(crate) fn check_wiring(&self) -> Result<(), TelemetryError> {
    self.shared.reset_probes();

    // Field names must be literals in the `tracing` macros, so they cannot
    // come from `PROBE_FIELD`; the test module keeps both in sync.
    let _probe_span = tracing::info_span!(
      "telemetry_probe",
      agent_id = "probe",
      telemetry_probe = true
    );
    tracing::event!(
      target: PERSIST_TARGET,
      Level::INFO,
      telemetry_probe = true,
      "telemetry wiring probe"
    );

    if !self.shared.probe_span_seen() {
      return Err(TelemetryError::WiringCheckFailed(
        "the probe span did not reach the layer; compose the layer, avoid \
         global filters that drop INFO, and make sure try_init() succeeded",
      ));
    }

    if !self.shared.probe_event_seen() {
      return Err(TelemetryError::WiringCheckFailed(
        "the probe event did not reach the layer; compose the layer, avoid \
         global filters that drop INFO, and make sure try_init() succeeded",
      ));
    }

    Ok(())
  }

  /// Dispatches what is already buffered and waits for confirmation.
  ///
  /// Everything enqueued before the call is attempted before the report is
  /// produced. `Ok` only means the batch was attempted; use
  /// [`FlushReport::is_complete`] to know whether something was lost.
  ///
  /// # Errors
  ///
  /// Returns [`TelemetryError::ChannelClosed`] when the runtime was already
  /// shut down.
  pub(crate) async fn flush(&self) -> Result<FlushReport, TelemetryError> {
    if self.shared.is_closed() {
      return Err(TelemetryError::ChannelClosed);
    }

    let (reply, report) = oneshot::channel();
    self
      .sender
      .send(Msg::Flush(reply))
      .await
      .map_err(|_| TelemetryError::ChannelClosed)?;

    report.await.map_err(|_| TelemetryError::ChannelClosed)
  }

  /// Dispatches what is pending and stops the worker.
  ///
  /// This is the only clean way to close the runtime. Consumes the handle.
  ///
  /// # Errors
  ///
  /// Returns [`TelemetryError::ChannelClosed`] when the worker is already
  /// gone.
  pub(crate) async fn shutdown(self) -> Result<FlushReport, TelemetryError> {
    self.shared.close();

    let (reply, report) = oneshot::channel();
    self
      .sender
      .send(Msg::Shutdown(reply))
      .await
      .map_err(|_| TelemetryError::ChannelClosed)?;

    report.await.map_err(|_| TelemetryError::ChannelClosed)
  }

  /// Returns a snapshot of the counters.
  #[must_use]
  pub(crate) fn stats(&self) -> TelemetryStats {
    self.shared.stats()
  }

  /// Starts the worker and returns the handle.
  fn spawn(config: TelemetryConfig) -> Self {
    let shared = Arc::new(Shared::new());
    let (sender, receiver) = mpsc::channel(config.channel_capacity);

    let state = WorkerState {
      sinks: config.sinks,
      retry: config.retry,
      batch_size: config.batch_size,
      flush_interval: config.flush_interval,
      shared: Arc::clone(&shared),
      buffer: Vec::with_capacity(config.batch_size),
      reported_delivered: 0,
      reported_lost: 0,
    };

    tokio::spawn(run_worker(state, receiver));

    Self { sender, shared }
  }
}

impl Drop for Telemetry {
  fn drop(&mut self) {
    if self.shared.is_closed() {
      return;
    }

    self.shared.close();

    let (reply, _report) = oneshot::channel();
    let request_sent = self.sender.try_send(Msg::Shutdown(reply)).is_ok();
    let pending = self
      .shared
      .enqueued()
      .saturating_sub(self.shared.processed());

    if pending > 0 {
      tracing::warn!(
        target: INTERNAL_TARGET,
        pending,
        request_sent,
        "telemetry was dropped with records still pending"
      );
    }
  }
}

/// State and configuration owned by the worker task.
struct WorkerState {
  sinks: Vec<Arc<dyn TraceSink>>,
  retry: RetryPolicy,
  batch_size: usize,
  flush_interval: Duration,
  shared: Arc<Shared>,
  buffer: Vec<TraceRecord>,
  reported_delivered: u64,
  reported_lost: u64,
}

impl WorkerState {
  /// Adds a record and reports whether a batch is ready to be dispatched.
  fn push(&mut self, record: TraceRecord) -> bool {
    self.buffer.push(record);
    self.buffer.len() >= self.batch_size
  }

  /// Dispatches an incomplete batch when the interval expires.
  async fn on_tick(&mut self) {
    if !self.buffer.is_empty() {
      self.dispatch().await;
    }
  }

  /// Fans the buffer out to every sink and clears it.
  async fn dispatch(&mut self) {
    if self.buffer.is_empty() {
      return;
    }

    let count = u64::try_from(self.buffer.len()).unwrap_or(u64::MAX);
    let batch = self.buffer.as_slice();
    let attempts = self.sinks.iter().map(|sink| {
      AssertUnwindSafe(write_with_retry(sink.as_ref(), batch, &self.retry))
        .catch_unwind()
    });
    let results = join_all(attempts).await;

    for (sink, result) in self.sinks.iter().zip(results) {
      match result {
        Ok(Ok(())) => self.shared.add_delivered(count),
        Ok(Err(error)) => {
          self.shared.add_lost(count);
          self.shared.add_failed_batch();
          tracing::error!(
            target: INTERNAL_TARGET,
            sink = sink.name(),
            records = count,
            error = %error,
            "telemetry sink failed to write a batch"
          );
        }
        Err(_panic) => {
          // A panic is contained here so a broken sink cannot take the
          // worker down with it; it is treated as a permanent failure.
          self.shared.add_lost(count);
          self.shared.add_failed_batch();
          tracing::error!(
            target: INTERNAL_TARGET,
            sink = sink.name(),
            records = count,
            "telemetry sink panicked"
          );
        }
      }
    }

    self.buffer.clear();
    self.shared.add_processed(count);
  }

  /// Builds the report of what happened since the previous call.
  fn take_report(&mut self) -> FlushReport {
    let delivered = self.shared.delivered();
    let lost = self.shared.lost_records();
    let report = FlushReport {
      delivered: delivered.saturating_sub(self.reported_delivered),
      lost: lost.saturating_sub(self.reported_lost),
    };

    self.reported_delivered = delivered;
    self.reported_lost = lost;
    report
  }
}

/// Sends a report, tolerating a caller that dropped its receiver.
///
/// The future waiting for the report may have been cancelled, so a closed
/// receiver is expected and not an error.
fn send_report(reply: oneshot::Sender<FlushReport>, report: FlushReport) {
  if reply.send(report).is_err() {
    tracing::trace!(
      target: INTERNAL_TARGET,
      "telemetry report was not received"
    );
  }
}

/// Drains the channel until a shutdown request arrives or every sender is
/// gone.
async fn run_worker(mut state: WorkerState, mut receiver: mpsc::Receiver<Msg>) {
  let mut ticker = tokio::time::interval(state.flush_interval);
  // The first tick of an interval fires immediately; consuming it here keeps
  // the periodic tick aligned with `flush_interval`.
  ticker.tick().await;

  loop {
    tokio::select! {
      message = receiver.recv() => {
        match message {
          None => break,
          Some(Msg::Record(record)) => {
            if state.push(record) {
              state.dispatch().await;
            }
          }
          Some(Msg::Flush(reply)) => {
            state.dispatch().await;
            let report = state.take_report();
            send_report(reply, report);
          }
          Some(Msg::Shutdown(reply)) => {
            state.dispatch().await;
            let report = state.take_report();
            send_report(reply, report);
            return;
          }
        }
      }
      _ = ticker.tick() => state.on_tick().await,
    }
  }
}

#[cfg(test)]
mod tests {
  use std::sync::atomic::Ordering;
  use std::sync::{Arc, Mutex};
  use std::time::Duration;

  use async_trait::async_trait;
  use rstest::rstest;
  use tokio::sync::Notify;
  use tracing::field::{Field, Visit};
  use tracing::{Event, Subscriber};
  use tracing_subscriber::layer::Context as LayerContext;
  use tracing_subscriber::layer::SubscriberExt as _;
  use tracing_subscriber::{Layer, registry};

  use super::super::{
    DEFAULT_BATCH_SIZE, DEFAULT_CHANNEL_CAPACITY, DEFAULT_FLUSH_INTERVAL,
    FailKind, INTERNAL_TARGET, MemorySink, RetryPolicy, SinkError, TraceLevel,
    TraceRecord, TraceSink,
  };
  use super::{
    FlushReport, Msg, Telemetry, TelemetryBuilder, TelemetryError,
    TelemetryStats,
  };
  use crate::telemetry::{agent_span, persist_info};
  use tracing_subscriber::EnvFilter;

  /// Long interval, so that time-driven flushes never interfere.
  const NEVER: Duration = Duration::from_hours(1);

  /// An event captured from the subscriber, with its target.
  #[derive(Debug, Default)]
  struct Captured {
    events: Mutex<Vec<(String, String)>>,
  }

  impl Captured {
    fn targets(&self) -> Vec<String> {
      self
        .events
        .lock()
        .unwrap()
        .iter()
        .map(|(target, _)| target.clone())
        .collect()
    }

    fn messages(&self) -> Vec<String> {
      self
        .events
        .lock()
        .unwrap()
        .iter()
        .map(|(_, message)| message.clone())
        .collect()
    }
  }

  /// Layer that records every event it sees, whatever its target.
  #[derive(Debug)]
  struct CaptureLayer {
    captured: Arc<Captured>,
  }

  impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: LayerContext<'_, S>) {
      let mut visitor = MessageVisitor::default();
      event.record(&mut visitor);
      self
        .captured
        .events
        .lock()
        .unwrap()
        .push((event.metadata().target().to_owned(), visitor.message));
    }
  }

  /// Visitor that only keeps the message of an event.
  #[derive(Debug, Default)]
  struct MessageVisitor {
    message: String,
  }

  impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
      if field.name() == "message" {
        self.message = format!("{value:?}");
      }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
      if field.name() == "message" {
        value.clone_into(&mut self.message);
      }
    }
  }

  /// Behaviour of a test sink.
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  enum Mode {
    /// Accepts every batch.
    Healthy,
    /// Panics on every batch.
    Panic,
    /// Logs with `tracing` on every batch.
    Emit,
    /// Blocks on the first batch until the test releases it.
    Gate,
  }

  /// Sink with a configurable behaviour, used to exercise the worker.
  #[derive(Debug)]
  struct TestSink {
    name: &'static str,
    mode: Mode,
    batches: Mutex<Vec<usize>>,
    entered: Notify,
    release: Notify,
  }

  impl TestSink {
    fn new(name: &'static str, mode: Mode) -> Self {
      Self {
        name,
        mode,
        batches: Mutex::new(Vec::new()),
        entered: Notify::new(),
        release: Notify::new(),
      }
    }

    /// Records the size of the batch and returns the call number.
    fn record(&self, size: usize) -> usize {
      let mut batches = self.batches.lock().unwrap();
      batches.push(size);
      batches.len()
    }

    fn batch_sizes(&self) -> Vec<usize> {
      self.batches.lock().unwrap().clone()
    }
  }

  #[async_trait]
  impl TraceSink for TestSink {
    fn name(&self) -> &str {
      self.name
    }

    async fn write_batch(
      &self,
      batch: &[TraceRecord],
    ) -> Result<(), SinkError> {
      let call = self.record(batch.len());

      match self.mode {
        Mode::Healthy => Ok(()),
        Mode::Panic => panic!("test sink panicked"),
        Mode::Emit => {
          tracing::info!("a sink writing its own log");
          Ok(())
        }
        Mode::Gate => {
          if call == 1 {
            self.entered.notify_one();
            self.release.notified().await;
          }
          Ok(())
        }
      }
    }
  }

  /// Telemetry, sink and the subscriber that wires both together.
  struct Harness {
    telemetry: Telemetry,
    sink: MemorySink,
    dispatch: tracing::Dispatch,
    captured: Arc<Captured>,
  }

  /// Erases the concrete sink type.
  fn share<T: TraceSink + 'static>(sink: Arc<T>) -> Arc<dyn TraceSink> {
    sink
  }

  /// Builds a `Telemetry` that writes into the returned handle.
  fn memory_telemetry(
    batch_size: usize,
    interval: Duration,
  ) -> (Telemetry, MemorySink) {
    let sink = MemorySink::unbounded();
    let handle = sink.clone();
    let telemetry = Telemetry::builder()
      .sink(sink)
      .batch_size(batch_size)
      .flush_interval(interval)
      .build()
      .expect("telemetry builds");

    (telemetry, handle)
  }

  /// Wires `telemetry` into a subscriber, with an extra capturing layer.
  fn harness(telemetry: Telemetry, sink: MemorySink) -> Harness {
    let captured = Arc::new(Captured::default());
    let dispatch = tracing::Dispatch::new(
      registry()
        .with(CaptureLayer {
          captured: Arc::clone(&captured),
        })
        .with(telemetry.layer()),
    );

    Harness {
      telemetry,
      sink,
      dispatch,
      captured,
    }
  }

  /// Builds the common case: a memory sink and a wired subscriber.
  fn wired(batch_size: usize, interval: Duration) -> Harness {
    let (telemetry, sink) = memory_telemetry(batch_size, interval);
    harness(telemetry, sink)
  }

  /// Builds a record with the given sequence number.
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

  #[test]
  fn builder_defaults_match_the_documented_constants() {
    // Arrange
    let builder = TelemetryBuilder::default().sink(MemorySink::new());

    // Act
    let config = builder.build_config().expect("config is valid");

    // Assert
    assert_eq!(config.channel_capacity, DEFAULT_CHANNEL_CAPACITY);
    assert_eq!(config.batch_size, DEFAULT_BATCH_SIZE);
    assert_eq!(config.flush_interval, DEFAULT_FLUSH_INTERVAL);
    assert_eq!(config.retry, RetryPolicy::default());
  }

  #[test]
  fn build_outside_a_runtime_returns_no_runtime() {
    // Arrange
    let builder = Telemetry::builder().sink(MemorySink::new());

    // Act
    let result = builder.build();

    // Assert
    assert!(matches!(result, Err(TelemetryError::NoRuntime)));
  }

  #[tokio::test]
  async fn build_without_a_sink_returns_invalid_config() {
    // Arrange
    let builder = Telemetry::builder();

    // Act
    let result = builder.build();

    // Assert
    assert!(matches!(result, Err(TelemetryError::InvalidConfig(_))));
  }

  #[rstest]
  #[case(0, 1, Duration::from_secs(1))]
  #[case(1, 0, Duration::from_secs(1))]
  #[case(1, 1, Duration::ZERO)]
  #[tokio::test]
  async fn invalid_values_return_invalid_config(
    #[case] channel_capacity: usize,
    #[case] batch_size: usize,
    #[case] flush_interval: Duration,
  ) {
    // Arrange
    let builder = Telemetry::builder()
      .sink(MemorySink::new())
      .channel_capacity(channel_capacity)
      .batch_size(batch_size)
      .flush_interval(flush_interval);

    // Act
    let result = builder.build();

    // Assert
    assert!(matches!(result, Err(TelemetryError::InvalidConfig(_))));
  }

  #[tokio::test]
  async fn in_memory_returns_a_working_handle() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = tracing::Dispatch::new(
      registry()
        .with(CaptureLayer {
          captured: Arc::new(Captured::default()),
        })
        .with(telemetry.layer()),
    );
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    agent_span!("agent-1", "session-a").in_scope(|| persist_info!("hello"));
    let report = telemetry.flush().await.expect("flush");

    // Assert
    assert_eq!(
      report,
      FlushReport {
        delivered: 1,
        lost: 0
      }
    );
    assert!(report.is_complete());
    assert_eq!(sink.len(), 1);
  }

  #[tokio::test]
  async fn second_layer_is_inert_and_does_not_duplicate_records() {
    // Arrange
    let captured = Arc::new(Captured::default());
    let telemetry = Telemetry::builder()
      .sink(MemorySink::new())
      .build()
      .expect("build");
    let dispatch = tracing::Dispatch::new(registry().with(CaptureLayer {
      captured: Arc::clone(&captured),
    }));
    let _guard = tracing::dispatcher::set_default(&dispatch);
    let first = telemetry.layer();

    // Act
    let second = telemetry.layer();
    let subscriber = registry().with(first).with(second);
    let _composed =
      tracing::dispatcher::set_default(&tracing::Dispatch::new(subscriber));
    agent_span!("agent-1", "session-a").in_scope(|| persist_info!("once"));
    telemetry.flush().await.expect("flush");

    // Assert
    let mut targets = captured.targets();
    targets.retain(|target| target == INTERNAL_TARGET);
    assert_eq!(targets.len(), 1);
  }

  #[tokio::test]
  async fn flush_returns_zeroed_report_when_nothing_is_pending() {
    // Arrange
    let harness = wired(1, NEVER);
    let _guard = tracing::dispatcher::set_default(&harness.dispatch);

    // Act
    let report = harness.telemetry.flush().await.expect("flush");

    // Assert
    assert_eq!(report, FlushReport::default());
    assert!(report.is_complete());
  }

  #[tokio::test]
  async fn worker_dispatches_when_the_batch_is_full() {
    // Arrange
    let probe = Arc::new(TestSink::new("probe", Mode::Gate));
    let handle = share(Arc::clone(&probe));
    let telemetry = Telemetry::builder()
      .sink_arc(handle)
      .batch_size(2)
      .flush_interval(NEVER)
      .build()
      .expect("build");
    let harness = harness(telemetry, MemorySink::new());

    // Act
    harness
      .telemetry
      .sender
      .send(Msg::Record(record(1)))
      .await
      .expect("send");
    harness
      .telemetry
      .sender
      .send(Msg::Record(record(2)))
      .await
      .expect("send");
    // Blocks until the worker is inside `write_batch`, which proves the batch
    // was dispatched because it was full, with no flush and no interval.
    probe.entered.notified().await;

    // Assert
    assert_eq!(probe.batch_sizes(), vec![2]);

    // Cleanup
    probe.release.notify_one();
    harness.telemetry.shutdown().await.expect("shutdown");
  }

  #[tokio::test(start_paused = true)]
  async fn a_batch_one_short_of_the_batch_size_is_not_dispatched() {
    // Arrange
    let harness = wired(3, NEVER);
    tokio::task::yield_now().await;

    // Act
    for seq in 1..=2 {
      harness
        .telemetry
        .sender
        .send(Msg::Record(record(seq)))
        .await
        .expect("send");
    }
    tokio::task::yield_now().await;

    // Assert
    assert!(harness.sink.is_empty());

    // Act: a flush proves the records were buffered rather than lost.
    harness.telemetry.flush().await.expect("flush");
    assert_eq!(harness.sink.len(), 2);
  }

  #[tokio::test(start_paused = true)]
  async fn worker_dispatches_an_incomplete_batch_when_the_interval_expires() {
    // Arrange
    let harness = wired(4, Duration::from_secs(1));
    tokio::task::yield_now().await;
    harness
      .telemetry
      .sender
      .send(Msg::Record(record(1)))
      .await
      .expect("send");

    // Act
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;

    // Assert
    assert_eq!(harness.sink.len(), 1);
  }

  #[tokio::test(start_paused = true)]
  async fn idle_worker_never_touches_the_sink() {
    // Arrange
    let sink = MemorySink::new();
    let batches = Arc::new(TestSink::new("probe", Mode::Healthy));
    let handle = share(Arc::clone(&batches));
    let telemetry = Telemetry::builder()
      .sink(sink)
      .sink_arc(handle)
      .batch_size(4)
      .flush_interval(Duration::from_secs(1))
      .build()
      .expect("build");
    let _harness = harness(telemetry, MemorySink::new());
    tokio::task::yield_now().await;

    // Act
    tokio::time::advance(Duration::from_secs(5)).await;
    tokio::task::yield_now().await;

    // Assert
    assert_eq!(batches.batch_sizes().len(), 0);
  }

  #[tokio::test(start_paused = true)]
  async fn large_input_is_split_into_batches_of_at_most_the_batch_size() {
    // Arrange
    let probe = Arc::new(TestSink::new("probe", Mode::Healthy));
    let handle = share(Arc::clone(&probe));
    let telemetry = Telemetry::builder()
      .sink_arc(handle)
      .batch_size(2)
      .flush_interval(NEVER)
      .build()
      .expect("build");
    let harness = harness(telemetry, MemorySink::new());
    tokio::task::yield_now().await;

    // Act
    for seq in 1..=5 {
      harness
        .telemetry
        .sender
        .send(Msg::Record(record(seq)))
        .await
        .expect("send");
    }
    harness.telemetry.flush().await.expect("flush");

    // Assert
    assert_eq!(probe.batch_sizes(), vec![2, 2, 1]);
  }

  #[tokio::test]
  async fn flush_preserves_the_order_of_the_records() {
    // Arrange
    let harness = wired(8, NEVER);
    tokio::task::yield_now().await;

    // Act
    for seq in 1..=4 {
      harness
        .telemetry
        .sender
        .send(Msg::Record(record(seq)))
        .await
        .expect("send");
    }
    harness.telemetry.flush().await.expect("flush");

    // Assert
    let seqs: Vec<u64> = harness
      .sink
      .records()
      .iter()
      .map(|record| record.seq)
      .collect();
    assert_eq!(seqs, vec![1, 2, 3, 4]);
  }

  #[tokio::test]
  async fn shutdown_delivers_everything_pending() {
    // Arrange
    let harness = wired(8, NEVER);
    tokio::task::yield_now().await;
    harness
      .telemetry
      .sender
      .send(Msg::Record(record(1)))
      .await
      .expect("send");

    // Act
    let report = harness.telemetry.shutdown().await.expect("shutdown");

    // Assert
    assert_eq!(report.delivered, 1);
    assert!(report.is_complete());
    assert_eq!(harness.sink.len(), 1);
  }

  #[tokio::test]
  async fn fan_out_sends_every_record_to_every_sink() {
    // Arrange
    let first = MemorySink::unbounded();
    let second = MemorySink::unbounded();
    let telemetry = Telemetry::builder()
      .sink(first.clone())
      .sink(second.clone())
      .batch_size(2)
      .flush_interval(NEVER)
      .build()
      .expect("build");
    let harness = harness(telemetry, MemorySink::new());
    tokio::task::yield_now().await;

    // Act
    harness
      .telemetry
      .sender
      .send(Msg::Record(record(1)))
      .await
      .expect("send");
    harness
      .telemetry
      .sender
      .send(Msg::Record(record(2)))
      .await
      .expect("send");
    let report = harness.telemetry.flush().await.expect("flush");

    // Assert
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2);
    assert_eq!(report.delivered, 4);
    assert!(report.is_complete());
  }

  #[tokio::test]
  async fn a_failing_sink_does_not_stop_the_others() {
    // Arrange
    let failing = MemorySink::new();
    failing.fail_always(FailKind::Permanent);
    let healthy = MemorySink::unbounded();
    let telemetry = Telemetry::builder()
      .sink(failing)
      .sink(healthy.clone())
      .batch_size(1)
      .flush_interval(NEVER)
      .build()
      .expect("build");
    let harness = harness(telemetry, MemorySink::new());
    tokio::task::yield_now().await;

    // Act
    harness
      .telemetry
      .sender
      .send(Msg::Record(record(1)))
      .await
      .expect("send");
    let report = harness.telemetry.flush().await.expect("flush");

    // Assert
    assert_eq!(healthy.len(), 1);
    assert_eq!(report.delivered, 1);
    assert_eq!(report.lost, 1);
    assert!(!report.is_complete());
    let stats = harness.telemetry.stats();
    assert_eq!(stats.failed_batches, 1);
    assert_eq!(stats.lost_records, 1);
  }

  #[tokio::test]
  async fn a_batch_that_recovers_after_retries_is_delivered() {
    // Arrange
    let sink = MemorySink::unbounded();
    sink.fail_next(2, FailKind::Transient);
    let telemetry = Telemetry::builder()
      .sink(sink.clone())
      .batch_size(1)
      .flush_interval(NEVER)
      .retry(
        RetryPolicy::new(
          3,
          Duration::from_millis(1),
          Duration::from_millis(1),
          1,
        )
        .expect("valid policy"),
      )
      .build()
      .expect("build");
    let harness = harness(telemetry, MemorySink::new());
    tokio::task::yield_now().await;

    // Act
    harness
      .telemetry
      .sender
      .send(Msg::Record(record(1)))
      .await
      .expect("send");
    let report = harness.telemetry.flush().await.expect("flush");

    // Assert
    assert_eq!(report.delivered, 1);
    assert!(report.is_complete());
    assert_eq!(sink.call_count(), 3);
  }

  #[tokio::test]
  async fn a_panicking_sink_is_contained() {
    // Arrange
    let healthy = MemorySink::unbounded();
    let panicking = Arc::new(TestSink::new("panicking", Mode::Panic));
    let handle = share(Arc::clone(&panicking));
    let telemetry = Telemetry::builder()
      .sink(healthy.clone())
      .sink_arc(handle)
      .batch_size(1)
      .flush_interval(NEVER)
      .build()
      .expect("build");
    let harness = harness(telemetry, MemorySink::new());
    tokio::task::yield_now().await;

    // Act
    harness
      .telemetry
      .sender
      .send(Msg::Record(record(1)))
      .await
      .expect("send");
    harness
      .telemetry
      .sender
      .send(Msg::Record(record(2)))
      .await
      .expect("send");
    let report = harness.telemetry.flush().await.expect("flush");

    // Assert
    assert_eq!(healthy.len(), 2);
    assert_eq!(report.delivered, 2);
    assert_eq!(report.lost, 2);
    assert_eq!(harness.telemetry.stats().failed_batches, 2);
  }

  #[tokio::test]
  async fn a_sink_logging_from_write_batch_creates_no_records() {
    // Arrange
    let healthy = MemorySink::unbounded();
    let logging = Arc::new(TestSink::new("logging", Mode::Emit));
    let handle = share(Arc::clone(&logging));
    let telemetry = Telemetry::builder()
      .sink(healthy.clone())
      .sink_arc(handle)
      .batch_size(1)
      .flush_interval(NEVER)
      .build()
      .expect("build");
    let harness = harness(telemetry, MemorySink::new());
    let _guard = tracing::dispatcher::set_default(&harness.dispatch);
    tokio::task::yield_now().await;

    // Act
    harness
      .telemetry
      .sender
      .send(Msg::Record(record(1)))
      .await
      .expect("send");
    harness.telemetry.flush().await.expect("flush");

    // Assert
    assert_eq!(healthy.len(), 1);
    assert_eq!(harness.telemetry.stats().enqueued, 0);
  }

  #[tokio::test]
  async fn exhausted_retries_emit_an_internal_error_event() {
    // Arrange
    let sink = MemorySink::new();
    sink.fail_always(FailKind::Permanent);
    let telemetry = Telemetry::builder()
      .sink(sink)
      .batch_size(1)
      .flush_interval(NEVER)
      .build()
      .expect("build");
    let harness = harness(telemetry, MemorySink::new());
    let _guard = tracing::dispatcher::set_default(&harness.dispatch);
    tokio::task::yield_now().await;

    // Act
    harness
      .telemetry
      .sender
      .send(Msg::Record(record(1)))
      .await
      .expect("send");
    harness.telemetry.flush().await.expect("flush");

    // Assert
    assert!(
      harness
        .captured
        .targets()
        .iter()
        .any(|target| target == INTERNAL_TARGET)
    );
  }

  #[tokio::test]
  async fn a_gated_sink_delays_the_next_batch_of_the_other_sink() {
    // Arrange
    let fast = MemorySink::unbounded();
    let gated = Arc::new(TestSink::new("gated", Mode::Gate));
    let handle = share(Arc::clone(&gated));
    let telemetry = Telemetry::builder()
      .sink(fast.clone())
      .sink_arc(handle)
      .batch_size(1)
      .channel_capacity(1)
      .flush_interval(NEVER)
      .build()
      .expect("build");
    let harness = harness(telemetry, MemorySink::new());

    // Act
    harness
      .telemetry
      .sender
      .try_send(Msg::Record(record(1)))
      .expect("the channel has room");
    gated.entered.notified().await;

    // Assert
    assert_eq!(fast.len(), 1);
    harness
      .telemetry
      .sender
      .try_send(Msg::Record(record(2)))
      .expect("the channel has room");
    assert!(
      harness
        .telemetry
        .sender
        .try_send(Msg::Record(record(3)))
        .is_err()
    );
    // The worker is still stuck in the gated sink, so the second record has
    // not reached the fast sink yet.
    assert_eq!(fast.len(), 1);

    // Act
    gated.release.notify_one();
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    // Assert
    assert_eq!(fast.len(), 2);
  }

  #[tokio::test]
  async fn check_wiring_succeeds_with_a_per_layer_filter() {
    // Arrange
    let (telemetry, sink) = memory_telemetry(8, NEVER);
    let subscriber = registry()
      .with(
        tracing_subscriber::fmt::layer()
          .with_writer(std::io::sink)
          .with_filter(EnvFilter::from_default_env()),
      )
      .with(telemetry.layer());
    let dispatch = tracing::Dispatch::new(subscriber);
    let _guard = tracing::dispatcher::set_default(&dispatch);

    // Act
    let result = telemetry.check_wiring();

    // Assert
    assert!(result.is_ok());
    assert_eq!(telemetry.stats(), TelemetryStats::default());
    assert!(sink.is_empty());
  }

  #[tokio::test]
  async fn check_wiring_fails_with_a_global_filter_that_drops_info() {
    // Arrange
    let (telemetry, _sink) = memory_telemetry(8, NEVER);
    let dispatch = tracing::Dispatch::new(
      registry()
        .with(EnvFilter::new("error"))
        .with(telemetry.layer()),
    );
    let _guard = tracing::dispatcher::set_default(&dispatch);

    // Act
    let result = telemetry.check_wiring();

    // Assert
    assert!(matches!(result, Err(TelemetryError::WiringCheckFailed(_))));
  }

  #[tokio::test]
  async fn check_wiring_fails_when_the_layer_is_not_composed() {
    // Arrange
    let (telemetry, _sink) = memory_telemetry(8, NEVER);
    let dispatch = tracing::Dispatch::new(
      registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::sink)),
    );
    let _guard = tracing::dispatcher::set_default(&dispatch);

    // Act
    let result = telemetry.check_wiring();

    // Assert
    assert!(matches!(result, Err(TelemetryError::WiringCheckFailed(_))));
  }

  #[tokio::test]
  async fn check_wiring_fails_without_an_active_subscriber() {
    // Arrange
    let (telemetry, _sink) = memory_telemetry(8, NEVER);

    // Act
    let first = telemetry.check_wiring();
    let second = telemetry.check_wiring();

    // Assert
    assert!(matches!(first, Err(TelemetryError::WiringCheckFailed(_))));
    assert!(matches!(second, Err(TelemetryError::WiringCheckFailed(_))));
  }

  #[tokio::test]
  async fn check_wiring_can_be_called_twice() {
    // Arrange
    let harness = wired(8, NEVER);
    let _guard = tracing::dispatcher::set_default(&harness.dispatch);

    // Act
    let first = harness.telemetry.check_wiring();
    let second = harness.telemetry.check_wiring();

    // Assert
    assert!(first.is_ok());
    assert!(second.is_ok());
  }

  #[tokio::test]
  async fn layer_can_be_composed_into_a_dispatch() {
    // Arrange
    let (telemetry, _sink) = memory_telemetry(8, NEVER);

    // Act: `Dispatch::new` requires `Send + Sync + 'static`, so building it
    // is itself the assertion.
    let dispatch = tracing::Dispatch::new(registry().with(telemetry.layer()));

    // Assert
    assert!(dispatch.downcast_ref::<registry::Registry>().is_some());
  }

  #[tokio::test]
  async fn stats_report_what_the_layer_accepted_and_dropped() {
    // Arrange
    let harness = wired(8, NEVER);
    let _guard = tracing::dispatcher::set_default(&harness.dispatch);
    agent_span!("agent-1", "session-a").in_scope(|| {
      for _ in 0..3 {
        persist_info!("counted");
      }
    });

    // Act
    harness.telemetry.flush().await.expect("flush");

    // Assert
    let stats = harness.telemetry.stats();
    assert_eq!(stats.enqueued, 3);
    assert_eq!(stats.delivered, 3);
    assert_eq!(stats.dropped, 0);
    assert_eq!(stats.missing_context, 0);
  }

  #[tokio::test]
  async fn events_after_shutdown_are_dropped() {
    // Arrange
    let harness = wired(8, NEVER);
    let _guard = tracing::dispatcher::set_default(&harness.dispatch);
    let shared = Arc::clone(&harness.telemetry.shared);
    harness.telemetry.shutdown().await.expect("shutdown");

    // Act
    agent_span!("agent-1", "session-a").in_scope(|| persist_info!("late"));

    // Assert
    assert_eq!(shared.dropped.load(Ordering::Relaxed), 1);
  }

  #[tokio::test]
  async fn flush_after_shutdown_returns_channel_closed() {
    // Arrange
    let harness = wired(8, NEVER);
    let handle = Telemetry {
      sender: harness.telemetry.sender.clone(),
      shared: Arc::clone(&harness.telemetry.shared),
    };
    harness.telemetry.shutdown().await.expect("shutdown");

    // Act
    let result = handle.flush().await;

    // Assert
    assert_eq!(result, Err(TelemetryError::ChannelClosed));
  }

  #[tokio::test]
  async fn dropping_with_pending_records_warns_and_does_not_panic() {
    // Arrange
    let harness = wired(8, NEVER);
    let _guard = tracing::dispatcher::set_default(&harness.dispatch);
    agent_span!("agent-1", "session-a").in_scope(|| persist_info!("pending"));

    // Act
    drop(harness.telemetry);

    // Assert
    assert!(
      harness
        .captured
        .messages()
        .iter()
        .any(|message| message.contains("still pending"))
    );
  }

  #[tokio::test]
  async fn dropping_without_pending_records_does_not_warn() {
    // Arrange
    let harness = wired(8, NEVER);
    let _guard = tracing::dispatcher::set_default(&harness.dispatch);

    // Act
    drop(harness.telemetry);

    // Assert
    assert!(
      !harness
        .captured
        .messages()
        .iter()
        .any(|message| message.contains("still pending"))
    );
  }

  #[tokio::test]
  async fn events_after_drop_are_dropped() {
    // Arrange
    let harness = wired(8, NEVER);
    let _guard = tracing::dispatcher::set_default(&harness.dispatch);
    let shared = Arc::clone(&harness.telemetry.shared);
    drop(harness.telemetry);

    // Act
    agent_span!("agent-1", "session-a").in_scope(|| persist_info!("late"));

    // Assert
    assert_eq!(shared.dropped.load(Ordering::Relaxed), 1);
  }

  #[tokio::test]
  async fn stats_snapshot_is_copyable_and_default() {
    // Arrange
    let stats = TelemetryStats::default();

    // Act
    let copy = stats;

    // Assert
    assert_eq!(copy, stats);
  }
}
