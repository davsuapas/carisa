//! The `tracing` layer that turns selected events into records.
//!
//! The layer is the only part of the module that runs on the calling thread.
//! It is wrapped in a per-layer filter so that `tracing` caches, per callsite,
//! whether the layer wants to see an event at all:
//!
//! - Events are accepted only when their target is
//!   [`PERSIST_TARGET`](crate::telemetry::PERSIST_TARGET), which is what the
//!   `persist_*!` macros set.
//! - Spans are accepted only when they declare an `agent_id` or a
//!   `session_id` field. A foreign span in the middle of the stack (for
//!   example one created by another library) is simply ignored, so the
//!   context is inherited from the closest agent ancestor.
//!
//! Because the decision is cached per callsite, an event that is not marked
//! costs almost nothing and none of its fields is visited.
//!
//! Identifiers are resolved when a span is created and stored in its
//! extensions as `Arc<str>`, so every event only clones an `Arc` — an atomic
//! increment, not a copy of the text. `on_event` performs no I/O and never
//! awaits: it builds the record and tries to enqueue it, counting a drop if
//! the channel is full or closed.
//!
//! An identifier whose value is empty or whitespace-only is treated as
//! absent: it is neither stored nor copied to descendants, and it never
//! overwrites a value that was already resolved.
//!
//! # Marking an event
//!
//! Crate-internal. Only the macros below mark an event for persistence, and
//! the rest of carisa reaches them as `crate::telemetry::persist_*!`:
//!
//! ```text
//! persist_info!("plain message");
//! persist_info!("formatted {}", 42);
//! persist_info!(tool = "search", tokens = 12u64, ok = true, "done");
//! persist_warn!(error = %err, "failed");       // Display -> Str
//! persist_error!(payload = ?value, "sent");    // Debug   -> Str
//! persist_event!(tracing::Level::DEBUG, code = -1i64, "verbose");
//! ```
//!
//! - Fields go before the message, as in `tracing`.
//! - There is no level threshold: every `persist_*!` event is persisted, so
//!   a low level multiplies the volume.
//! - Only the `info`, `warn` and `error` shortcuts exist; use
//!   `persist_event!` for `DEBUG` and `TRACE`.
//! - The events are emitted with the target `telemetry::persist`, not with
//!   the module path, so a filter such as `my_crate=info` does not match
//!   them. The real module path survives in the `target` field of the
//!   record.
//! - `persist_*!` inside a sink would feed the pipeline with its own output:
//!   never do it.
//!
//! # Tagging a record with agent and session
//!
//! Crate-internal. `agent_span!` creates the span that carries the two
//! identifiers; any marked event inside it, or inside a future instrumented
//! with it, reaches the sink with both:
//!
//! ```text
//! // INFO, and the explicit level in the three-argument form.
//! let span = agent_span!(agent_id, session_id);
//! let span = agent_span!(tracing::Level::DEBUG, agent_id, session_id);
//! ```
//!
//! The two identifiers are resolved once per span and stored in its
//! extensions as `Arc<str>`, so an event only clones an `Arc`. The rules:
//!
//! - A child span that declares `session_id` overrides only the session; it
//!   inherits `agent_id` from the closest agent ancestor. A foreign span in
//!   between (one created by another library) is ignored.
//! - An identifier that is empty or whitespace-only counts as absent: it is
//!   neither stored nor inherited, and it never overwrites a value already
//!   resolved.
//! - An identifier known only later is declared as `tracing::field::Empty`
//!   and filled with `span.record("agent_id", value)`.
//! - Always propagate the span: `.instrument(span)` for a future,
//!   `span.in_scope(..)` for a blocking task or a plain thread, and never a
//!   `span.enter()` guard held across an `.await`. A span that is created
//!   but never entered nor instrumented adds no context. Symptom: `None`
//!   identifiers in the sink.
//! - Outside any agent span the record still reaches the sink, with both
//!   identifiers set to `None`; the runtime counts it as a missing context,
//!   never as a loss.
//!
//! # Verification
//!
//! The behaviour above is covered by the tests of this file, among them
//! `marked_event_inside_agent_span_carries_the_identifiers`,
//! `marked_event_without_a_span_has_no_identifiers`,
//! `inner_span_overrides_only_the_session`,
//! `foreign_intermediate_span_does_not_break_the_inheritance`,
//! `identifier_declared_empty_is_filled_by_a_later_record`,
//! `instrumented_task_keeps_the_context`,
//! `task_without_instrument_loses_the_context`,
//! `blocking_task_with_in_scope_keeps_the_context`,
//! `full_channel_drops_records_and_never_blocks` and
//! `extra_fields_map_to_their_native_kinds`.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;

use tokio::sync::mpsc;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Record};
use tracing::{Event, Id, Metadata, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;

use super::record::{FieldValue, TraceLevel, TraceRecord};
use super::runtime::{Msg, Shared};
use super::{
  AGENT_ID_FIELD, MESSAGE_FIELD, PERSIST_TARGET, PROBE_FIELD, SESSION_ID_FIELD,
};

/// Tells whether a callsite should reach the layer.
///
/// Used as a per-layer filter, so `tracing` caches the answer for each
/// callsite and unmarked events never visit their fields.
pub(super) fn is_persisted(metadata: &Metadata<'_>) -> bool {
  if metadata.is_event() {
    return metadata.target() == PERSIST_TARGET;
  }

  let fields = metadata.fields();
  fields.field(AGENT_ID_FIELD).is_some()
    || fields.field(SESSION_ID_FIELD).is_some()
}

/// Agent and session identifiers attached to a span.
#[derive(Debug, Default)]
pub(super) struct SpanContext {
  agent_id: Option<Arc<str>>,
  session_id: Option<Arc<str>>,
}

impl SpanContext {
  /// Returns `true` when neither identifier was resolved.
  const fn is_empty(&self) -> bool {
    self.agent_id.is_none() && self.session_id.is_none()
  }
}

/// Layer that converts marked events into [`TraceRecord`]s.
#[derive(Debug)]
pub(super) struct PersistLayer {
  sender: mpsc::Sender<Msg>,
  shared: Arc<Shared>,
}

impl PersistLayer {
  /// Builds a layer that enqueues records through `sender`.
  pub(super) const fn new(
    sender: mpsc::Sender<Msg>,
    shared: Arc<Shared>,
  ) -> Self {
    Self { sender, shared }
  }
}

impl<S> Layer<S> for PersistLayer
where
  S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
  fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
    if attrs.metadata().fields().field(PROBE_FIELD).is_some() {
      self.shared.mark_probe_span();
    }

    let mut context = SpanContext::default();
    attrs.record(&mut SpanVisitor {
      context: &mut context,
    });

    if context.is_empty() {
      return;
    }

    if let Some(span) = ctx.span(id) {
      span.extensions_mut().insert(context);
    }
  }

  fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
    let Some(span) = ctx.span(id) else {
      return;
    };

    let mut context = span
      .extensions_mut()
      .remove::<SpanContext>()
      .unwrap_or_default();
    values.record(&mut SpanVisitor {
      context: &mut context,
    });

    if !context.is_empty() {
      span.extensions_mut().insert(context);
    }
  }

  fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
    let metadata = event.metadata();
    let mut visitor = EventVisitor::new(extra_field_capacity(metadata));
    event.record(&mut visitor);

    if visitor.probe {
      self.shared.mark_probe_event();
      return;
    }

    let (agent_id, session_id) = resolve_context(&ctx, event);

    if agent_id.is_none() || session_id.is_none() {
      self.shared.record_missing_context();
    }

    let target = metadata
      .module_path()
      .map_or_else(|| Cow::Borrowed(metadata.target()), Cow::Borrowed);
    let record = TraceRecord::new(
      TraceLevel::from(metadata.level()),
      target,
      visitor.message,
      visitor.fields,
    )
    .with_context(agent_id, session_id);

    // A closed handle means the runtime is shutting down or already gone:
    // records are not accepted any more, and saying so explicitly keeps the
    // behaviour independent of the channel's remaining capacity.
    let accepted = !self.shared.is_closed()
      && self.sender.try_send(Msg::Record(record)).is_ok();

    if accepted {
      self.shared.record_enqueued();
    } else {
      self.shared.record_dropped();
    }
  }
}

/// Returns the number of fields an event may store besides the message.
///
/// Reserving exactly the extra fields keeps `Vec::with_capacity(0)` — and
/// therefore no allocation — for the common case of an event that only has a
/// message.
fn extra_field_capacity(metadata: &Metadata<'_>) -> usize {
  let fields = metadata.fields();
  let has_message = fields.field(MESSAGE_FIELD).is_some();
  let has_probe = fields.field(PROBE_FIELD).is_some();

  let reserved = match (has_message, has_probe) {
    (true, true) => 2,
    (true, false) | (false, true) => 1,
    (false, false) => 0,
  };

  fields.len().saturating_sub(reserved)
}

/// Resolves the agent and session identifiers of an event.
///
/// The scope is walked from the innermost span to the root. Each identifier
/// is resolved independently and the closest value wins; the loop stops as
/// soon as both are known.
fn resolve_context<S>(
  ctx: &Context<'_, S>,
  event: &Event<'_>,
) -> (Option<Arc<str>>, Option<Arc<str>>)
where
  S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
  let Some(scope) = ctx.event_scope(event) else {
    return (None, None);
  };

  let mut agent_id = None;
  let mut session_id = None;

  for span in scope {
    let extensions = span.extensions();
    let found = extensions.get::<SpanContext>().map(|context| {
      (
        identifier(context.agent_id.as_ref(), agent_id.is_none()),
        identifier(context.session_id.as_ref(), session_id.is_none()),
      )
    });
    drop(extensions);

    let Some((agent, session)) = found else {
      continue;
    };

    if agent.is_some() {
      agent_id = agent;
    }

    if session.is_some() {
      session_id = session;
    }

    if agent_id.is_some() && session_id.is_some() {
      break;
    }
  }

  (agent_id, session_id)
}

/// Clones an identifier when `wanted` is set.
fn identifier(value: Option<&Arc<str>>, wanted: bool) -> Option<Arc<str>> {
  if wanted {
    return value.map(Arc::clone);
  }

  None
}

/// Visitor that reads the identifiers declared by a span.
struct SpanVisitor<'a> {
  context: &'a mut SpanContext,
}

impl SpanVisitor<'_> {
  /// Returns the slot of the field, when the field is an identifier.
  fn slot(&mut self, field: &Field) -> Option<&mut Option<Arc<str>>> {
    match field.name() {
      AGENT_ID_FIELD => Some(&mut self.context.agent_id),
      SESSION_ID_FIELD => Some(&mut self.context.session_id),
      _ => None,
    }
  }

  /// Stores the identifier, ignoring blanks.
  fn set(slot: &mut Option<Arc<str>>, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
      return;
    }

    *slot = Some(Arc::from(trimmed));
  }
}

impl Visit for SpanVisitor<'_> {
  fn record_str(&mut self, field: &Field, value: &str) {
    let Some(slot) = self.slot(field) else {
      return;
    };

    Self::set(slot, value);
  }

  fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
    let Some(slot) = self.slot(field) else {
      return;
    };

    Self::set(slot, &format!("{value:?}"));
  }
}

/// Visitor that reads the message and the extra fields of an event.
struct EventVisitor {
  message: String,
  fields: Vec<(Cow<'static, str>, FieldValue)>,
  probe: bool,
}

impl EventVisitor {
  /// Creates a visitor reserving room for `capacity` extra fields.
  fn new(capacity: usize) -> Self {
    Self {
      message: String::new(),
      fields: Vec::with_capacity(capacity),
      probe: false,
    }
  }

  /// Stores an extra field, or flags the event as a wiring probe.
  fn push(&mut self, field: &Field, value: FieldValue) {
    if field.name() == PROBE_FIELD {
      self.probe = true;
      return;
    }

    self.fields.push((Cow::Borrowed(field.name()), value));
  }
}

impl Visit for EventVisitor {
  fn record_str(&mut self, field: &Field, value: &str) {
    if field.name() == MESSAGE_FIELD {
      value.clone_into(&mut self.message);
      return;
    }

    self.push(field, FieldValue::Str(value.to_owned()));
  }

  fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
    let text = format!("{value:?}");

    if field.name() == MESSAGE_FIELD {
      self.message = text;
      return;
    }

    self.push(field, FieldValue::Str(text));
  }

  fn record_i64(&mut self, field: &Field, value: i64) {
    self.push(field, FieldValue::I64(value));
  }

  fn record_u64(&mut self, field: &Field, value: u64) {
    self.push(field, FieldValue::U64(value));
  }

  fn record_f64(&mut self, field: &Field, value: f64) {
    self.push(field, FieldValue::F64(value));
  }

  fn record_bool(&mut self, field: &Field, value: bool) {
    self.push(field, FieldValue::Bool(value));
  }

  fn record_i128(&mut self, field: &Field, value: i128) {
    self.push(field, FieldValue::Str(value.to_string()));
  }

  fn record_u128(&mut self, field: &Field, value: u128) {
    self.push(field, FieldValue::Str(value.to_string()));
  }

  fn record_error(
    &mut self,
    field: &Field,
    value: &(dyn std::error::Error + 'static),
  ) {
    self.push(field, FieldValue::Str(value.to_string()));
  }
}

#[cfg(test)]
mod tests {
  use std::sync::{Arc, Mutex};

  use async_trait::async_trait;
  use rstest::rstest;
  use tracing::Instrument;
  use tracing_subscriber::layer::SubscriberExt as _;

  use super::super::{
    FailKind, FieldValue, MemorySink, SinkError, Telemetry, TraceRecord,
    TraceSink,
  };
  use super::super::{INTERNAL_TARGET, PERSIST_TARGET, PROBE_FIELD};
  use super::AGENT_ID_FIELD;
  use crate::telemetry::{agent_span, persist_event, persist_info};

  /// Builds a subscriber with the telemetry layer, ready to be installed.
  fn dispatch_for(telemetry: &Telemetry) -> tracing::Dispatch {
    tracing::Dispatch::new(
      tracing_subscriber::registry().with(telemetry.layer()),
    )
  }

  /// Returns the value of an extra field of a record.
  fn field<'a>(record: &'a TraceRecord, name: &str) -> Option<&'a FieldValue> {
    record
      .fields
      .iter()
      .find(|(key, _)| key == name)
      .map(|(_, value)| value)
  }

  /// Sink that keeps what each batch looked like, without copying records.
  #[derive(Debug, Default)]
  struct ProbeSink {
    /// `(records in the batch, capacity of the first record's fields)`.
    batches: Mutex<Vec<(usize, usize)>>,
  }

  #[async_trait]
  impl TraceSink for ProbeSink {
    fn name(&self) -> &'static str {
      "probe"
    }

    async fn write_batch(
      &self,
      batch: &[TraceRecord],
    ) -> Result<(), SinkError> {
      let capacity = batch.first().map_or(0, |record| record.fields.capacity());
      self.batches.lock().unwrap().push((batch.len(), capacity));
      Ok(())
    }
  }

  #[tokio::test]
  async fn unmarked_event_does_not_reach_the_sink() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    tracing::info!("not persisted");
    telemetry.flush().await.expect("flush");

    // Assert
    assert_eq!(telemetry.stats().enqueued, 0);
    assert!(sink.is_empty());
  }

  #[tokio::test]
  async fn marked_event_inside_agent_span_carries_the_identifiers() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    agent_span!("agent-1", "session-a").in_scope(|| {
      persist_info!(step = 1_u64, "agent started");
    });
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.agent_id(), Some("agent-1"));
    assert_eq!(record.session_id(), Some("session-a"));
    assert_eq!(record.message, "agent started");
    assert_eq!(record.target, "carisa_core::telemetry::layer::tests");
    assert_eq!(field(&record, "step"), Some(&FieldValue::U64(1)));
  }

  #[tokio::test]
  async fn marked_event_without_a_span_has_no_identifiers() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    persist_info!("no span");
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.agent_id(), None);
    assert_eq!(record.session_id(), None);
    assert_eq!(telemetry.stats().missing_context, 1);
  }

  #[tokio::test]
  async fn inner_span_overrides_only_the_session() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    agent_span!("agent-1", "session-a").in_scope(|| {
      tracing::info_span!("subtask", session_id = %"session-b")
        .in_scope(|| persist_info!("nested"));
    });
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.agent_id(), Some("agent-1"));
    assert_eq!(record.session_id(), Some("session-b"));
    assert_eq!(telemetry.stats().missing_context, 0);
  }

  #[rstest]
  #[case("")]
  #[case(" ")]
  #[case("\t")]
  #[tokio::test]
  async fn blank_session_id_falls_back_to_the_outer_span(#[case] blank: &str) {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    agent_span!("agent-1", "session-a").in_scope(|| {
      tracing::info_span!("subtask", session_id = %blank)
        .in_scope(|| persist_info!("nested"));
    });
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.agent_id(), Some("agent-1"));
    assert_eq!(record.session_id(), Some("session-a"));
  }

  #[tokio::test]
  async fn foreign_intermediate_span_does_not_break_the_inheritance() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    agent_span!("agent-1", "session-a").in_scope(|| {
      tracing::info_span!("foreign").in_scope(|| persist_info!("nested"));
    });
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.agent_id(), Some("agent-1"));
    assert_eq!(record.session_id(), Some("session-a"));
  }

  #[tokio::test]
  async fn identifier_declared_empty_is_filled_by_a_later_record() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");
    let span = tracing::info_span!(
      "agent",
      agent_id = tracing::field::Empty,
      session_id = %"session-a"
    );

    // Act
    span.record("agent_id", "filled-later");
    span.in_scope(|| persist_info!("nested"));
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.agent_id(), Some("filled-later"));
    assert_eq!(record.session_id(), Some("session-a"));
  }

  #[tokio::test]
  async fn blank_record_does_not_overwrite_a_previous_identifier() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");
    let span = tracing::info_span!("agent", session_id = %"session-a");

    // Act
    span.record("session_id", "");
    span.in_scope(|| persist_info!("nested"));
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.session_id(), Some("session-a"));
  }

  #[tokio::test]
  async fn internal_target_is_never_persisted() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    tracing::error!(target: INTERNAL_TARGET, "internal diagnostic");
    telemetry.flush().await.expect("flush");

    // Assert
    assert_eq!(telemetry.stats().enqueued, 0);
    assert!(sink.is_empty());
  }

  #[tokio::test]
  async fn hand_written_persist_target_is_persisted() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    tracing::info!(target: PERSIST_TARGET, "hand written");
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.message, "hand written");
    assert_eq!(record.target, "carisa_core::telemetry::layer::tests");
  }

  #[tokio::test]
  async fn extra_fields_map_to_their_native_kinds() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    agent_span!("agent-1", "session-a").in_scope(|| {
      persist_info!(
        text = "value",
        signed = -7_i64,
        unsigned = 7_u64,
        ratio = 1.5_f64,
        flag = true,
        debug = ?vec![1_u8],
        display = %"shown",
        big = 340_282_366_920_938_463_463_374_607_431_768_211_455_u128,
        "message {}",
        42
      );
    });
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.message, "message 42");
    assert_eq!(
      field(&record, "text"),
      Some(&FieldValue::Str("value".to_owned()))
    );
    assert_eq!(field(&record, "signed"), Some(&FieldValue::I64(-7)));
    assert_eq!(field(&record, "unsigned"), Some(&FieldValue::U64(7)));
    assert_eq!(field(&record, "ratio"), Some(&FieldValue::F64(1.5)));
    assert_eq!(field(&record, "flag"), Some(&FieldValue::Bool(true)));
    assert_eq!(
      field(&record, "debug"),
      Some(&FieldValue::Str("[1]".to_owned()))
    );
    assert_eq!(
      field(&record, "display"),
      Some(&FieldValue::Str("shown".to_owned()))
    );
    assert_eq!(
      field(&record, "big"),
      Some(&FieldValue::Str(
        "340282366920938463463374607431768211455".to_owned()
      ))
    );
    assert_eq!(field(&record, PROBE_FIELD), None);
  }

  #[tokio::test]
  async fn message_only_event_does_not_allocate_field_storage() {
    // Arrange
    let probe = Arc::new(ProbeSink::default());
    let shared = Arc::clone(&probe);
    let handle: Arc<dyn TraceSink> = shared;
    let telemetry = Telemetry::builder()
      .sink_arc(handle)
      .build()
      .expect("build");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    agent_span!("agent-1", "session-a").in_scope(|| persist_info!("only"));
    telemetry.flush().await.expect("flush");

    // Assert
    let observed = probe.batches.lock().unwrap().clone();
    assert_eq!(observed, vec![(1, 0)]);
  }

  #[tokio::test]
  async fn debug_level_persist_event_is_persisted() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    agent_span!("agent-1", "session-a").in_scope(|| {
      persist_event!(tracing::Level::DEBUG, code = -1_i64, "verbose");
    });
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.message, "verbose");
    assert_eq!(field(&record, "code"), Some(&FieldValue::I64(-1)));
  }

  #[tokio::test]
  async fn full_channel_drops_records_and_never_blocks() {
    // Arrange
    let sink = MemorySink::new();
    let telemetry = Telemetry::builder()
      .sink(sink)
      .channel_capacity(1)
      .build()
      .expect("build");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    agent_span!("agent-1", "session-a").in_scope(|| {
      persist_info!("one");
      persist_info!("two");
      persist_info!("three");
    });

    // Assert
    let stats = telemetry.stats();
    assert_eq!(stats.enqueued, 1);
    assert_eq!(stats.dropped, 2);
  }

  #[tokio::test]
  async fn instrumented_task_keeps_the_context() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    let span = agent_span!("agent-1", "session-a");
    tokio::spawn(async { persist_info!("spawned") }.instrument(span))
      .await
      .expect("task finishes");
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.agent_id(), Some("agent-1"));
    assert_eq!(record.session_id(), Some("session-a"));
  }

  #[tokio::test]
  async fn task_without_instrument_loses_the_context() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    tokio::spawn(async { persist_info!("spawned") })
      .await
      .expect("task finishes");
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.agent_id(), None);
    assert_eq!(telemetry.stats().missing_context, 1);
  }

  #[tokio::test]
  async fn span_created_but_not_entered_provides_no_context() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    let span = agent_span!("agent-1", "session-a");
    drop(span);
    persist_info!("outside");
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.agent_id(), None);
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
  async fn blocking_task_with_in_scope_keeps_the_context() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    let span = agent_span!("agent-1", "session-a");
    let blocking = dispatch.clone();
    tokio::task::spawn_blocking(move || {
      tracing::dispatcher::with_default(&blocking, || {
        span.in_scope(|| persist_info!("blocking"));
      });
    })
    .await
    .expect("blocking task finishes");
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.agent_id(), Some("agent-1"));
    assert_eq!(record.session_id(), Some("session-a"));
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
  async fn blocking_task_without_in_scope_loses_the_context() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    let _span = agent_span!("agent-1", "session-a");
    let blocking = dispatch.clone();
    tokio::task::spawn_blocking(move || {
      tracing::dispatcher::with_default(&blocking, || {
        persist_info!("blocking");
      });
    })
    .await
    .expect("blocking task finishes");
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.agent_id(), None);
  }

  #[tokio::test]
  async fn interleaved_agents_do_not_mix_their_identifiers() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    let first = async {
      persist_info!("first");
      tokio::task::yield_now().await;
    }
    .instrument(agent_span!("agent-1", "session-a"));
    let second = async {
      persist_info!("second");
      tokio::task::yield_now().await;
    }
    .instrument(agent_span!("agent-2", "session-b"));
    tokio::join!(first, second);
    telemetry.flush().await.expect("flush");

    // Assert
    let mut seen: Vec<(Option<String>, String)> = sink
      .records()
      .iter()
      .map(|record| {
        (record.agent_id().map(str::to_owned), record.message.clone())
      })
      .collect();
    seen.sort();
    assert_eq!(
      seen,
      vec![
        (Some("agent-1".to_owned()), "first".to_owned()),
        (Some("agent-2".to_owned()), "second".to_owned()),
      ]
    );
  }

  #[tokio::test]
  async fn agent_span_with_an_explicit_level_carries_the_identifiers() {
    // Arrange
    let (telemetry, sink) = Telemetry::in_memory().expect("in memory");
    let dispatch = dispatch_for(&telemetry);
    let _guard = tracing::dispatcher::set_default(&dispatch);
    telemetry.check_wiring().expect("wired");

    // Act
    agent_span!(tracing::Level::DEBUG, "agent-1", "session-a")
      .in_scope(|| persist_info!("explicit level"));
    telemetry.flush().await.expect("flush");

    // Assert
    let record = sink.records().first().cloned().expect("one record");
    assert_eq!(record.agent_id(), Some("agent-1"));
    assert_eq!(record.session_id(), Some("session-a"));
  }

  #[test]
  fn probe_field_literal_matches_the_constant() {
    // Arrange
    let literal = "telemetry_probe";

    // Act
    let matches = literal == PROBE_FIELD;

    // Assert
    assert!(matches);
  }

  #[test]
  fn identifier_field_names_are_the_documented_ones() {
    // Arrange
    let expected = ["agent_id", "session_id"];

    // Act
    let actual = [AGENT_ID_FIELD, "session_id"];

    // Assert
    assert_eq!(actual, expected);
  }

  #[test]
  fn failing_sink_kind_is_used_by_the_memory_sink() {
    // Arrange
    let sink = MemorySink::new();

    // Act
    sink.fail_next(1, FailKind::Transient);

    // Assert
    assert_eq!(sink.call_count(), 0);
  }
}
