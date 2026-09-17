//! Persistence of complete responses produced by a Rig stream.

use std::sync::Arc;

use futures_util::{Stream, StreamExt};
use rig_core::{
  completion::message::ReasoningContent, streaming::StreamedAssistantContent,
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
/// reasoning/tool events, and stores the full batch in the configured log
/// storage.
///
/// Returns the IDs of the persisted log records.
///
/// # Examples
///
/// With callbacks for live streaming:
///
/// ```rust
/// use futures_util::stream;
/// use rig_core::streaming::StreamedAssistantContent;
///
/// # async fn example(storage: std::sync::Arc<dyn crate::memory::log::storage::LogStorage>) {
/// let on_user_delta = |delta: &str| println!("text: {delta}");
/// let on_thinking_delta = |delta: &str| println!("thinking: {delta}");
///
/// let items = vec![
///   Ok(StreamedAssistantContent::text("hello")),
///   Ok(StreamedAssistantContent::ReasoningDelta {
///     id: "r1".to_owned(),
///     provider_id: None,
///     reasoning: "thinking".to_owned(),
///   }),
///   Ok(StreamedAssistantContent::Final(
///     rig_core::streaming::StreamFinal::new(
///       "provider",
///       rig_core::completion::Usage::default(),
///     ),
///   )),
/// ];
///
/// let _ = crate::memory::log::streaming::process_stream_and_log(
///   stream::iter(items),
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
pub async fn process_stream_and_log<S, UserCallback, ThinkingCallback>(
  mut stream: S,
  agent_id: &str,
  session_id: &str,
  storage: Arc<dyn LogStorage>,
  on_user_delta: Option<UserCallback>,
  on_thinking_delta: Option<ThinkingCallback>,
) -> Result<Vec<String>, LogError>
where
  S: Stream<
      Item = Result<
        StreamedAssistantContent,
        rig_core::completion::CompletionError,
      >,
    > + Unpin,
  UserCallback: Fn(&str),
  ThinkingCallback: Fn(&str),
{
  let assistant_id = generate_message_id();
  let mut saved_ids = Vec::new();
  let mut assistant_events = Vec::new();
  let mut text = String::new();

  while let Some(item) = stream.next().await {
    let item = item.map_err(LogError::Completion)?;
    match item {
      StreamedAssistantContent::Text(delta) => {
        text.push_str(&delta.text);
        if let Some(callback) = &on_user_delta {
          callback(&delta.text);
        }
      }
      StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
        if let Some(callback) = &on_thinking_delta {
          callback(&reasoning);
        }
      }
      StreamedAssistantContent::Reasoning { reasoning, .. } => {
        assistant_events.push(new_message(
          agent_id,
          session_id,
          LogMessageKind::Thinking {
            reasoning: to_stored_reasoning(reasoning),
            parent_assistant_id: Some(assistant_id.clone()),
          },
        ));
      }
      StreamedAssistantContent::ToolCall { tool_call, .. } => {
        assistant_events.push(new_message(
          agent_id,
          session_id,
          LogMessageKind::ToolCall {
            tool_name: tool_call.function.name,
            arguments: tool_call.function.arguments,
            parent_assistant_id: Some(assistant_id.clone()),
            step_id: None,
          },
        ));
      }
      StreamedAssistantContent::ToolCallDelta { .. }
      | StreamedAssistantContent::Unknown(_) => {}
      StreamedAssistantContent::Final(final_response) => {
        let assistant = new_message_with_id(
          assistant_id.clone(),
          agent_id,
          session_id,
          LogMessageKind::Assistant {
            content: std::mem::take(&mut text),
            provider: Some(final_response.provider),
            model: final_response.model,
            provider_response_id: final_response
              .message_id
              .or(final_response.response_id),
          },
        );
        let mut messages = vec![assistant];
        messages.append(&mut assistant_events);
        saved_ids.extend(storage.append(agent_id, session_id, messages).await?);
      }
    }
  }

  Ok(saved_ids)
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

fn to_stored_reasoning(
  reasoning: rig_core::completion::message::Reasoning,
) -> StoredReasoning {
  StoredReasoning {
    id: reasoning.id,
    content: reasoning
      .content
      .into_iter()
      .map(to_stored_reasoning_content)
      .collect(),
  }
}

fn to_stored_reasoning_content(
  content: ReasoningContent,
) -> StoredReasoningContent {
  match content {
    ReasoningContent::Text { text, signature } => {
      StoredReasoningContent::Text { text, signature }
    }
    ReasoningContent::Encrypted(data) => {
      StoredReasoningContent::Encrypted(data)
    }
    ReasoningContent::Redacted { data } => {
      StoredReasoningContent::Redacted { data }
    }
    ReasoningContent::Summary(data) => StoredReasoningContent::Summary(data),
  }
}

#[cfg(test)]
mod tests {
  use std::sync::{Arc, Mutex};

  use futures_util::stream;
  use rig_core::{
    completion::message::{ToolCall, ToolCallId, ToolFunction},
    streaming::{StreamFinal, StreamedAssistantContent},
  };

  use super::process_stream_and_log;
  use crate::memory::log::{
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
    let tool_call = ToolCall::new(
      ToolCallId::new_or_mint("call"),
      ToolFunction::new(
        "lookup".to_owned(),
        serde_json::json!({"key": "value"}),
      ),
    );
    let items = vec![
      Ok(StreamedAssistantContent::text("hello")),
      Ok(StreamedAssistantContent::ReasoningDelta {
        id: "reasoning".to_owned(),
        provider_id: None,
        reasoning: "thinking".to_owned(),
      }),
      Ok(StreamedAssistantContent::Reasoning {
        id: "reasoning".to_owned(),
        reasoning: rig_core::completion::message::Reasoning::new("analysis"),
      }),
      Ok(StreamedAssistantContent::ToolCall {
        tool_call,
        internal_call_id: "internal".to_owned(),
      }),
      Ok(StreamedAssistantContent::Final(StreamFinal::new(
        "provider",
        rig_core::completion::Usage::default(),
      ))),
    ];

    let storage_for_process = storage_trait(Arc::clone(&storage));
    let ids = process_stream_and_log(
      stream::iter(items),
      "agent",
      "session",
      storage_for_process,
      Some(text_callback),
      Some(thinking_callback),
    )
    .await
    .expect("stream should be processed");
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
        parent_assistant_id: Some(parent_id),
        ..
      }) if parent_id == assistant_id
    ));
    assert_eq!(
      *text_deltas.lock().expect("text lock should work"),
      ["hello"]
    );
    assert_eq!(
      *thinking_deltas.lock().expect("thinking lock should work"),
      ["thinking"]
    );
  }

  fn storage_trait(storage: Arc<InMemoryLogStorage>) -> Arc<dyn LogStorage> {
    storage
  }
}
