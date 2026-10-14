//! Persistence of complete responses produced by a Rig stream.

use std::sync::Arc;

use futures_util::{Stream, StreamExt};
use rig_core::{
  completion::{
    CompletionResponse,
    message::{AssistantContent, Reasoning, ToolCall},
  },
  error::ProviderError,
  streaming::{CompletionStream, Item, StreamEvent},
};

use super::{
  error::LogError,
  storage::LogStorage,
  types::{
    LogMessage, LogMessageKind, StoredReasoning, StoredReasoningContent,
    generate_message_id, now_millis,
  },
};

/// Consumes a streamed Rig assistant response and persists the complete event
/// timeline for a single completion.
///
/// The stream can emit several different message types while the model is
/// generating its answer: text chunks, reasoning updates, tool calls, and final
/// metadata. This function keeps track of those events as they arrive, updating
/// the in-memory text buffer and collecting related reasoning/tool entries that
/// should be attached to the final assistant message.
///
/// Text deltas are appended to the final content and optionally forwarded to a
/// callback for live UI updates. Reasoning and tool-call events are recorded as
/// child log messages associated with the generated assistant message via
/// `parent_assistant_id`. Transient delta-only events such as
/// `ToolCallDelta` and `Unknown` are ignored because they do not represent a
/// complete persisted record.
///
/// Once the stream reaches `Final`, the function creates the assistant log entry
/// with the accumulated content and provider metadata, appends all associated
/// reasoning/tool events in the order their complete stream events arrived, and
/// stores the full batch in the configured log storage. The assistant record is
/// first, followed by those child events.
///
/// Returns the IDs of the persisted log records.
///
/// # Examples
///
/// With callbacks for live streaming:
///
/// ```rust
/// use rig_core::streaming::CompletionStream;
///
/// # async fn example(
/// #   completion: CompletionStream,
/// #   storage: std::sync::Arc<dyn carisa_core::memory::session::storage::LogStorage>,
/// # ) {
/// let on_user_delta = |delta: &str| println!("text: {delta}");
/// let on_thinking_delta = |delta: &str| println!("thinking: {delta}");
///
/// let _ = carisa_core::memory::session::streaming::process_stream_and_log(
///   completion,
///   "agent-1",
///   "session-1",
///   storage,
///   Some(on_user_delta),
///   Some(on_thinking_delta),
/// )
/// .await;
/// # }
/// ```
///
/// These callbacks are useful for updating a UI, console, or any live stream as
/// the model produces text and reasoning in real time.
///
/// # Errors
///
/// Returns an error if the underlying stream yields a completion failure, or if
/// the storage backend cannot persist one of the generated log entries.
pub async fn process_stream_and_log<UserCallback, ThinkingCallback>(
  mut stream: CompletionStream,
  agent_id: &str,
  session_id: &str,
  storage: Arc<dyn LogStorage>,
  on_user_delta: Option<UserCallback>,
  on_thinking_delta: Option<ThinkingCallback>,
) -> Result<Vec<String>, LogError>
where
  UserCallback: Fn(&str) + Send + Sync,
  ThinkingCallback: Fn(&str) + Send + Sync,
{
  let assistant_id = generate_message_id();
  let collected = collect_stream_events(
    &mut stream,
    &assistant_id,
    on_user_delta.as_ref(),
    on_thinking_delta.as_ref(),
  )
  .await?;
  let response = stream.partial();

  persist_stream_response(
    agent_id,
    session_id,
    storage,
    assistant_id,
    collected,
    CompletionMetadata::from(response),
  )
  .await
}

struct CollectedStream {
  text: String,
  events: Vec<LogMessageKind>,
}

struct CompletionMetadata {
  provider: String,
  model: Option<String>,
  provider_response_id: Option<String>,
}

impl From<CompletionResponse> for CompletionMetadata {
  fn from(response: CompletionResponse) -> Self {
    Self {
      provider: response.provider().to_owned(),
      model: response.model().map(str::to_owned),
      provider_response_id: response.response_id().map(str::to_owned),
    }
  }
}

async fn collect_stream_events<S, UserCallback, ThinkingCallback>(
  stream: &mut S,
  assistant_id: &str,
  on_user_delta: Option<&UserCallback>,
  on_thinking_delta: Option<&ThinkingCallback>,
) -> Result<CollectedStream, LogError>
where
  S: Stream<Item = Result<Item<StreamEvent>, ProviderError>> + Send + Unpin,
  UserCallback: Fn(&str) + Send + Sync,
  ThinkingCallback: Fn(&str) + Send + Sync,
{
  let mut collected = CollectedStream {
    text: String::new(),
    events: Vec::new(),
  };

  while let Some(item) = stream.next().await {
    match item.map_err(LogError::from)? {
      Item::Unknown(_) => {}
      Item::Event(event) => match event {
        StreamEvent::Text { text, .. } => {
          collected.text.push_str(&text);
          if let Some(callback) = on_user_delta {
            callback(&text);
          }
        }
        StreamEvent::Reasoning { text, .. } => {
          if let Some(callback) = on_thinking_delta {
            callback(&text);
          }
        }
        StreamEvent::End { content, .. } => match content {
          AssistantContent::Reasoning(reasoning) => {
            collected.events.push(LogMessageKind::Thinking {
              reasoning: to_stored_reasoning(&reasoning),
              parent_assistant_id: Some(assistant_id.to_owned()),
            });
          }
          AssistantContent::ToolCall(tool_call) => {
            collected
              .events
              .push(tool_call_log_event(tool_call, assistant_id));
          }
          _ => {}
        },
        StreamEvent::Start { .. } | StreamEvent::Arguments { .. } => {}
      },
    }
  }

  Ok(collected)
}

fn tool_call_log_event(
  tool_call: ToolCall,
  assistant_id: &str,
) -> LogMessageKind {
  LogMessageKind::ToolCall {
    tool_name: tool_call.function.name.to_string(),
    arguments: serde_json::Value::Object(tool_call.function.arguments),
    call_id: Some(tool_call.id.to_string()),
    parent_assistant_id: Some(assistant_id.to_owned()),
    step_id: None,
  }
}

async fn persist_stream_response(
  agent_id: &str,
  session_id: &str,
  storage: Arc<dyn LogStorage>,
  assistant_id: String,
  collected: CollectedStream,
  metadata: CompletionMetadata,
) -> Result<Vec<String>, LogError> {
  let assistant = new_message_with_id(
    assistant_id,
    agent_id,
    session_id,
    LogMessageKind::Assistant {
      content: collected.text,
      provider: Some(metadata.provider),
      model: metadata.model,
      provider_response_id: metadata.provider_response_id,
    },
  );
  let mut messages = vec![assistant];
  messages.extend(
    collected
      .events
      .into_iter()
      .map(|kind| new_message(agent_id, session_id, kind)),
  );
  storage.append(agent_id, session_id, messages).await
}

fn new_message(
  agent_id: &str,
  session_id: &str,
  kind: LogMessageKind,
) -> LogMessage {
  new_message_with_id(generate_message_id(), agent_id, session_id, kind)
}

fn new_message_with_id(
  id: String,
  agent_id: &str,
  session_id: &str,
  kind: LogMessageKind,
) -> LogMessage {
  LogMessage {
    id,
    agent_id: agent_id.to_owned(),
    agent_nickname: String::new(),
    session_id: session_id.to_owned(),
    timestamp: now_millis(),
    kind,
  }
}

fn to_stored_reasoning(reasoning: &Reasoning) -> StoredReasoning {
  let content = if reasoning.redacted {
    StoredReasoningContent::Redacted {
      data: reasoning.text.clone(),
    }
  } else {
    StoredReasoningContent::Text {
      text: reasoning.text.clone(),
      signature: None,
    }
  };
  StoredReasoning {
    id: None,
    content: vec![content],
    native: reasoning.native.as_ref().map(|native| native.item.clone()),
  }
}

#[cfg(test)]
mod tests {
  use std::sync::{Arc, Mutex};

  use futures_util::stream;
  use rig_core::{
    completion::message::{
      AssistantContent, Reasoning, ToolCall, ToolFunction,
    },
    message::{CallId, ToolName},
    streaming::{Item, StreamEvent, StreamRecord},
  };

  use super::{
    CompletionMetadata, collect_stream_events, persist_stream_response,
  };
  use crate::memory::session::{
    inmemory::InMemoryLogStorage, storage::LogStorage, types::LogMessageKind,
  };

  #[tokio::test]
  async fn persists_complete_events_and_emits_deltas() {
    let storage = Arc::new(InMemoryLogStorage::new());
    let text_deltas = Arc::new(Mutex::new(Vec::new()));
    let thinking_deltas = Arc::new(Mutex::new(Vec::new()));
    let text_callback = {
      let text_deltas = Arc::clone(&text_deltas);
      move |delta: &str| {
        text_deltas
          .lock()
          .expect("text lock should work")
          .push(delta.to_owned());
      }
    };
    let thinking_callback = {
      let thinking_deltas = Arc::clone(&thinking_deltas);
      move |delta: &str| {
        thinking_deltas
          .lock()
          .expect("thinking lock should work")
          .push(delta.to_owned());
      }
    };
    let items = stream_items();
    let mut stream = stream::iter(items.into_iter().map(Ok));
    let collected = collect_stream_events(
      &mut stream,
      "assistant",
      Some(&text_callback),
      Some(&thinking_callback),
    )
    .await
    .expect("stream should be collected");
    let ids = persist_stream_response(
      "agent",
      "session",
      storage_trait(Arc::clone(&storage)),
      "assistant".to_owned(),
      collected,
      CompletionMetadata {
        provider: "provider".to_owned(),
        model: Some("model".to_owned()),
        provider_response_id: Some("response".to_owned()),
      },
    )
    .await
    .expect("stream response should be persisted");
    let messages = storage
      .history("agent", "session")
      .await
      .expect("history should work");

    assert_eq!(ids.len(), 3);
    assert_eq!(messages.len(), 3);
    let assistant_id = messages
      .first()
      .expect("assistant message should exist")
      .id
      .as_str();
    assert!(matches!(
      messages.first().map(|message| &message.kind),
      Some(LogMessageKind::Assistant { .. })
    ));
    assert!(matches!(
      messages.get(1).map(|message| &message.kind),
      Some(LogMessageKind::Thinking {
        parent_assistant_id: Some(parent_id),
        ..
      }) if parent_id == assistant_id
    ));
    assert!(matches!(
      messages.get(2).map(|message| &message.kind),
      Some(LogMessageKind::ToolCall {
        call_id: Some(call_id),
        parent_assistant_id: Some(parent_id),
        ..
      }) if parent_id == assistant_id && call_id == "call"
    ));
    assert_eq!(
      *text_deltas.lock().expect("text lock should work"),
      ["hello"]
    );
    assert_eq!(
      *thinking_deltas.lock().expect("thinking lock should work"),
      ["thinking"]
    );
    assert!(matches!(
      messages.first().map(|message| &message.kind),
      Some(LogMessageKind::Assistant {
        provider: Some(provider),
        model: Some(model),
        provider_response_id: Some(response_id),
        ..
      }) if provider == "provider" && model == "model" && response_id == "response"
    ));
  }

  fn stream_items() -> Vec<Item<StreamEvent>> {
    let reasoning = AssistantContent::Reasoning(Reasoning {
      text: "analysis".to_owned(),
      redacted: false,
      native: None,
    });
    let tool_call = AssistantContent::ToolCall(ToolCall::new(
      CallId::from_wire("call"),
      ToolFunction::new(
        ToolName::new("lookup").expect("tool name should be valid"),
        serde_json::json!({"key": "value"}),
      ),
    ));
    let reasoning =
      serde_json::to_value(reasoning).expect("reasoning should serialize");
    let tool_call =
      serde_json::to_value(tool_call).expect("tool call should serialize");
    let transcript = StreamRecord::parse(serde_json::json!([
      {"item": "event", "value": {"event": "start", "part": 0, "kind": "text"}},
      {"item": "event", "value": {"event": "text", "part": 0, "text": "hello"}},
      {"item": "event", "value": {"event": "end", "part": 0, "content": {"type": "text", "text": "hello"}}},
      {"item": "event", "value": {"event": "start", "part": 1, "kind": "reasoning"}},
      {"item": "event", "value": {"event": "reasoning", "part": 1, "text": "thinking"}},
      {"item": "event", "value": {"event": "end", "part": 1, "content": reasoning}},
      {"item": "event", "value": {"event": "start", "part": 2, "kind": "tool_call", "name": "lookup"}},
      {"item": "event", "value": {"event": "end", "part": 2, "content": tool_call}}
    ]))
    .expect("transcript should be valid");

    transcript.into_items()
  }

  fn storage_trait(storage: Arc<InMemoryLogStorage>) -> Arc<dyn LogStorage> {
    storage
  }
}
