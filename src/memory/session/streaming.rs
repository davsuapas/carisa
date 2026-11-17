//! Persistence of complete responses produced by a Rig stream.

use std::sync::Arc;

use rig_core::{completion::CompletionResponse, streaming::CompletionStream};

use super::{
  error::LogError,
  storage::LogStorage,
  types::{LogMessage, MessageId},
};

/// Persists the assistant turn of a streamed completion.
///
/// The stream is finished with [`CompletionStream::finish`], which reads the
/// reply to its end and folds it into a [`CompletionResponse`]. Its
/// [`CompletionResponse::message`] is stored verbatim, provider items
/// included: those items are what let the same model replay the turn
/// unchanged, which keeps signed reasoning and the provider's prefix cache.
///
/// A reply that produced no content (a filtered or cut-short turn) stores
/// nothing.
///
/// # Examples
///
/// ```no_run
/// use std::sync::Arc;
///
/// use carisa_core::memory::session::{
///   storage::LogStorage, streaming::process_stream_and_log,
/// };
/// use rig_core::streaming::CompletionStream;
///
/// # async fn example(
/// #   stream: CompletionStream,
/// #   storage: Arc<dyn LogStorage>,
/// # ) -> Result<(), Box<dyn std::error::Error>> {
/// let ids = process_stream_and_log(stream, "agent-1", "session-1", storage)
///   .await?;
/// # let _ = ids;
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the stream ends without the provider's end, or if the
/// storage backend cannot persist the message.
pub async fn process_stream_and_log(
  stream: CompletionStream,
  agent_id: &str,
  session_id: &str,
  storage: Arc<dyn LogStorage>,
) -> Result<Vec<MessageId>, LogError> {
  let response = stream.finish().await?;
  let Some(message) = log_message(agent_id, session_id, &response) else {
    return Ok(Vec::new());
  };
  storage.append(agent_id, session_id, vec![message]).await
}

/// Builds the log record for `response`, or nothing when it holds no turn.
fn log_message(
  agent_id: &str,
  session_id: &str,
  response: &CompletionResponse,
) -> Option<LogMessage> {
  response
    .message()
    .map(|message| LogMessage::new(agent_id, session_id, message))
}

#[cfg(test)]
mod tests {
  use std::sync::Arc;

  use futures_util::stream;
  use rig_core::{
    completion::{
      CompletionResponse, FinishReason, Usage,
      message::{AssistantContent, ToolCall, ToolFunction},
    },
    error::{ErrorReport, ProviderError},
    message::{CallId, Origin, StopReason, ToolName},
    streaming::{CompletionStream, Relayed},
  };

  use super::{log_message, process_stream_and_log};
  use crate::memory::session::{
    error::LogError, inmemory::InMemoryLogStorage, storage::LogStorage,
    types::Etiqueta,
  };

  fn provider_item() -> serde_json::Value {
    serde_json::json!({"type": "text", "text": "hello"})
  }

  fn tool_call() -> AssistantContent {
    AssistantContent::ToolCall(ToolCall::new(
      CallId::from_wire("call-1"),
      ToolFunction::new(
        ToolName::new("lookup").expect("tool name should be valid"),
        serde_json::json!({"key": "value"}),
      ),
    ))
  }

  fn response(choice: Vec<AssistantContent>) -> CompletionResponse {
    CompletionResponse::new(
      choice,
      Usage::default(),
      Origin::new("test.chat", "test", "model"),
      serde_json::json!({}),
    )
    .with_finish_reason(FinishReason::Stop)
  }

  fn storage_trait(storage: Arc<InMemoryLogStorage>) -> Arc<dyn LogStorage> {
    storage
  }

  #[test]
  fn keeps_provider_items_and_metadata_of_the_turn() {
    let text = AssistantContent::text("hello").with_native(provider_item());
    let response = response(vec![text, tool_call()]);

    let log = log_message("agent", "session", &response)
      .expect("a non-empty turn should be logged");

    assert_eq!(log.etiqueta, Etiqueta::None);
    let rig_core::completion::Message::Assistant(turn) = &log.message else {
      panic!("expected an assistant message");
    };
    assert_eq!(turn.content.len(), 2);
    assert_eq!(
      turn.content.first().and_then(AssistantContent::native_item),
      Some(&provider_item())
    );
    assert_eq!(turn.stop, Some(StopReason::ToolUse));
    assert_eq!(
      turn.origin.as_ref().map(|origin| origin.provider.as_str()),
      Some("test")
    );
  }

  #[test]
  fn empty_choice_is_not_logged() {
    let response = response(Vec::new());

    assert!(log_message("agent", "session", &response).is_none());
  }

  #[tokio::test]
  async fn a_completed_stream_is_persisted_and_read_back() {
    let storage = Arc::new(InMemoryLogStorage::new());
    let response = response(vec![AssistantContent::text("hello")]);
    let events: Vec<Result<Relayed, ErrorReport>> =
      vec![Ok(Relayed::Done(Arc::new(response)))];
    let stream =
      CompletionStream::relay("test", Box::pin(stream::iter(events)));

    let ids = process_stream_and_log(
      stream,
      "agent",
      "session",
      storage_trait(Arc::clone(&storage)),
    )
    .await
    .expect("the completed stream should persist");

    assert_eq!(ids.len(), 1);
    let history = storage
      .history("agent", "session")
      .await
      .expect("history should work");
    let rig_core::completion::Message::Assistant(turn) =
      history.first().expect("one message should be stored")
    else {
      panic!("expected an assistant message");
    };
    assert_eq!(turn.content, vec![AssistantContent::text("hello")]);
  }

  #[tokio::test]
  async fn a_stream_without_the_provider_end_fails() {
    let storage = Arc::new(InMemoryLogStorage::new());
    let events: Vec<Result<Relayed, ErrorReport>> = Vec::new();
    let stream =
      CompletionStream::relay("test", Box::pin(stream::iter(events)));

    let result = process_stream_and_log(
      stream,
      "agent",
      "session",
      storage_trait(Arc::clone(&storage)),
    )
    .await;

    assert!(matches!(
      result,
      Err(LogError::Completion(error))
        if matches!(error.as_ref(), ProviderError::Truncated)
    ));
    assert_eq!(
      storage
        .history("agent", "session")
        .await
        .expect("history should work"),
      Vec::new()
    );
  }
}
