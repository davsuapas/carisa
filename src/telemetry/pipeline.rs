//! End-to-end tests of the whole telemetry pipeline.
//!
//! Test-only module: it is compiled with the tests of the crate, never into
//! the library. It lives inside the crate, not in `tests/`, because the
//! flows drive the crate-internal half of the module: the `persist_*!`
//! macros, the layer and the runtime lifecycle. Each test composes a real
//! subscriber and exercises the path an application sees once carisa wires
//! telemetry up.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tracing::Instrument as _;
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::{EnvFilter, Layer, registry};

use super::{MemorySink, Telemetry, TraceRecord};
use crate::telemetry::{agent_span, persist_info};

/// Long interval, so that time-driven flushes never interfere.
const NEVER: Duration = Duration::from_hours(1);

/// Number of events a subscriber has seen, whatever their target.
#[derive(Debug, Default)]
struct Counter {
  events: AtomicUsize,
}

/// Layer that counts every event, so a test can tell that the events
/// telemetry ignores still reach the rest of the subscriber.
#[derive(Debug)]
struct CountingLayer {
  counter: Arc<Counter>,
}

impl<S: tracing::Subscriber> Layer<S> for CountingLayer {
  fn on_event(
    &self,
    _event: &tracing::Event<'_>,
    _ctx: tracing_subscriber::layer::Context<'_, S>,
  ) {
    self.counter.events.fetch_add(1, Ordering::Relaxed);
  }
}

/// Installs a subscriber with a formatting layer and the telemetry layer.
///
/// The guard must be bound to a name: `let _ =` would drop it immediately and
/// uninstall the subscriber before the first event.
fn install(telemetry: &Telemetry) -> DefaultGuard {
  tracing::subscriber::set_default(
    registry()
      .with(
        tracing_subscriber::fmt::layer()
          .with_writer(std::io::sink)
          .with_filter(EnvFilter::from_default_env()),
      )
      .with(telemetry.layer()),
  )
}

/// Builds a subscriber with the counting layer and the telemetry layer.
fn counting_dispatch(
  counter: &Arc<Counter>,
  telemetry: &Telemetry,
) -> tracing::Dispatch {
  tracing::Dispatch::new(
    registry()
      .with(CountingLayer {
        counter: Arc::clone(counter),
      })
      .with(telemetry.layer()),
  )
}

#[tokio::test]
async fn full_flow_persists_only_the_marked_events() {
  // Arrange
  let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
  let _guard = install(&telemetry);
  telemetry.check_wiring().expect("wired");

  // Act
  tracing::info!("not persisted");
  agent_span!("agent-1", "session-a").in_scope(|| {
    persist_info!(step = 1_u64, "first");
  });
  run_agent("agent-2", "session-b").await;
  let report = telemetry.shutdown().await.expect("shutdown");

  // Assert
  assert!(report.is_complete());
  let records = sink.records();
  assert_eq!(records.len(), 2);

  let first = records.first().expect("first record");
  assert_eq!(first.agent_id(), Some("agent-1"));
  assert_eq!(first.session_id(), Some("session-a"));
  assert_eq!(first.message, "first");

  let second = records.get(1).expect("second record");
  assert_eq!(second.agent_id(), Some("agent-2"));
  assert_eq!(second.session_id(), Some("session-b"));
  assert_eq!(second.message, "second");
}

/// Emits one marked event inside an instrumented agent span.
async fn run_agent(agent: &str, session: &str) {
  let future = async { persist_info!("second") };
  future.instrument(agent_span!(agent, session)).await;
}

#[tokio::test]
async fn marked_and_unmarked_events_take_their_own_path() {
  // Arrange
  let counter = Arc::new(Counter::default());
  let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
  let dispatch = counting_dispatch(&counter, &telemetry);
  let _guard = tracing::dispatcher::set_default(&dispatch);
  telemetry.check_wiring().expect("wired");
  // Forget the wiring probes, so only the events below are counted.
  counter.events.store(0, Ordering::Relaxed);

  // Act
  tracing::info!("only the counting layer");
  agent_span!("agent-1", "session-a").in_scope(|| {
    persist_info!("both layers");
  });
  telemetry.flush().await.expect("flush");

  // Assert
  assert_eq!(counter.events.load(Ordering::Relaxed), 2);
  assert_eq!(telemetry.stats().enqueued, 1);
  let records = sink.records();
  assert_eq!(records.len(), 1);
  assert_eq!(
    records.first().map(|record| record.message.as_str()),
    Some("both layers")
  );
}

#[tokio::test]
async fn heavy_load_never_loses_a_record_on_a_healthy_sink() {
  // Arrange
  const EVENTS: u64 = 10_000;
  let sink = MemorySink::unbounded();
  let handle = sink.clone();
  let telemetry = Telemetry::builder()
    .sink(sink)
    .channel_capacity(16_384)
    .batch_size(256)
    .flush_interval(NEVER)
    .build()
    .expect("build");
  let _guard = install(&telemetry);
  telemetry.check_wiring().expect("wired");

  // Act
  agent_span!("agent-1", "session-a").in_scope(|| {
    for index in 0..EVENTS {
      persist_info!(index, "load");
    }
  });
  let enqueued = telemetry.stats().enqueued;
  let dropped = telemetry.stats().dropped;
  let report = telemetry.shutdown().await.expect("shutdown");

  // Assert
  assert_eq!(enqueued.saturating_add(dropped), EVENTS);
  assert_eq!(dropped, 0);
  assert_eq!(report.delivered, enqueued);
  assert!(report.is_complete());
  assert_eq!(u64::try_from(handle.len()).expect("fits"), enqueued);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn records_from_several_threads_have_unique_sequences() {
  // Arrange
  let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
  let dispatch = tracing::Dispatch::new(
    registry()
      .with(
        tracing_subscriber::fmt::layer()
          .with_writer(std::io::sink)
          .with_filter(EnvFilter::from_default_env()),
      )
      .with(telemetry.layer()),
  );
  let _guard = tracing::dispatcher::set_default(&dispatch);
  telemetry.check_wiring().expect("wired");

  // Act
  let handles: Vec<_> = (0..4)
    .map(|worker| {
      let dispatch = dispatch.clone();
      tokio::spawn(
        async move {
          // Every worker thread installs the same subscriber, so the events
          // it emits are attributed to the span it was instrumented with.
          let _guard = tracing::dispatcher::set_default(&dispatch);
          for _ in 0..50 {
            persist_info!(worker, "threaded");
          }
        }
        .instrument(agent_span!("agent-1", "session-a")),
      )
    })
    .collect();

  for handle in handles {
    handle.await.expect("worker finishes");
  }

  let report = telemetry.shutdown().await.expect("shutdown");

  // Assert
  let mut seqs: Vec<u64> = sink
    .records()
    .iter()
    .map(|record: &TraceRecord| record.seq)
    .collect();
  let total = seqs.len();
  assert_eq!(total, 200);
  seqs.sort_unstable();
  seqs.dedup();
  assert_eq!(seqs.len(), total);
  assert_eq!(report.delivered, 200);
}
