//! Projection of persisted logs into provider-agnostic messages.

use std::{fmt, sync::Arc};

use rig_core::completion::{
  Message,
  message::{
    AssistantContent, Reasoning, ReasoningContent, Text, ToolCall, ToolCallId,
    ToolFunction, ToolResult, ToolResultContent, UserContent,
  },
};

use super::types::{
  LogMessage, LogMessageKind, StoredReasoning, StoredReasoningContent,
};

/// Policy that decides which reasoning blocks enter the model history.
#[non_exhaustive]
pub enum ThinkingPolicy {
  /// Excludes all reasoning blocks.
  Never,
  /// Includes all valid reasoning blocks.
  Always,
  /// Includes reasoning when the predicate accepts the provider.
  Conditional(Arc<dyn Fn(&str) -> bool + Send + Sync>),
}

impl fmt::Debug for ThinkingPolicy {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Never => formatter.write_str("Never"),
      Self::Always => formatter.write_str("Always"),
      Self::Conditional(_) => formatter.write_str("Conditional(<function>)"),
    }
  }
}

/// Options applied while projecting a log.
#[non_exhaustive]
#[derive(Debug)]
pub struct ProjectionOptions {
  /// Reasoning inclusion policy.
  pub thinking_policy: ThinkingPolicy,
}

impl Default for ProjectionOptions {
  fn default() -> Self {
    Self {
      thinking_policy: ThinkingPolicy::Never,
    }
  }
}

/// Temporary accumulator that converts several consecutive log records into
/// a single Rig `Message::Assistant`.
///
/// In `LogMessage`, each event is persisted as an independent record: one
/// `Assistant` record contains the base text, another `ToolCall` record
/// contains a tool invocation, and another `Thinking` record contains the
/// reasoning. Rig, on the other hand, expects these contents to live inside a
/// single assistant message. This type keeps the identifier of the open
/// assistant record so it can accept only events whose
/// `parent_assistant_id` matches it.
///
/// When flushed, the accumulator produces this conceptual destination:
///
/// ```text
/// LogMessage(Assistant { content: "I will search" })
/// LogMessage(ToolCall { tool_name: "search", ... })
/// LogMessage(Thinking { reasoning: ... })
///                 |
///                 v
/// Message::Assistant {
///   content: [Text("I will search"), ToolCall(...), Reasoning(...)],
/// }
/// ```
struct PendingAssistant {
  id: String,
  content: String,
  provider: Option<String>,
  provider_response_id: Option<String>,
  tool_calls: Vec<ToolCall>,
  reasoning: Vec<StoredReasoning>,
}

impl PendingAssistant {
  const fn new(
    id: String,
    content: String,
    provider: Option<String>,
    provider_response_id: Option<String>,
  ) -> Self {
    Self {
      id,
      content,
      provider,
      provider_response_id,
      tool_calls: Vec::new(),
      reasoning: Vec::new(),
    }
  }
}

impl From<PendingAssistant> for Message {
  fn from(pending: PendingAssistant) -> Self {
    let mut content = vec![AssistantContent::Text(Text::new(pending.content))];
    content.extend(
      pending
        .tool_calls
        .into_iter()
        .map(AssistantContent::ToolCall),
    );
    content.extend(pending.reasoning.into_iter().map(|reasoning| {
      AssistantContent::Reasoning(to_rig_reasoning(reasoning))
    }));

    Self::Assistant {
      id: pending.provider_response_id,
      content,
    }
  }
}

/// Projects independent log records into Rig's message format.
///
/// The source and destination have different granularities:
///
/// - The log has one `LogMessage` per persisted record. For example, one
///   response may occupy three records: `Assistant`, `ToolCall`, and
///   `Thinking`.
/// - In Rig, those records must form a single `Message::Assistant`, whose
///   `content` contains several blocks: `Text`, `ToolCall`, and `Reasoning`.
///   Therefore, this function keeps a `PendingAssistant` while traversing the
///   sequence and converts it into a message only when it reaches a boundary.
///
/// The resulting organization follows these rules:
///
/// 1. An `Assistant` record opens a new group. If another group is already
///    open, the previous one is emitted before opening the new one.
/// 2. A `ToolCall` is added to the open group only when its
///    `parent_assistant_id` matches the `Assistant` record's id. A call with
///    no corresponding assistant is discarded.
/// 3. A `Thinking` record follows the same parent rule and also passes through
///    `ProjectionOptions::thinking_policy`. The relative order of calls and
///    reasoning records is preserved while they are accumulated, although Rig
///    represents them as blocks within the same assistant message.
/// 4. A `User`, `ToolResult`, or second `Assistant` record closes the pending
///    group before emitting its own message. Internal events (`Turn`, `Step`,
///    and `Condensation`) do not produce Rig messages.
///
/// For example, this log input:
///
/// ```text
/// Assistant(id=A, content="I am searching for data")
/// ToolCall(parent=A, tool_name="search", arguments={"q":"Rust"})
/// Thinking(parent=A, reasoning="...", allowed)
/// ToolResult(tool_call_id=call-1, result={"ok":true})
/// User(content="continue")
/// ```
///
/// is conceptually converted into:
///
/// ```text
/// Message::Assistant {
///   content: [Text("I am searching for data"), ToolCall(...), Reasoning(...)],
/// }
/// Message::User {
///   content: [ToolResult(call=call-1, text="{\"ok\":true}")],
/// }
/// Message::User {
///   content: [Text("continue")],
/// }
/// ```
///
/// The function does not reorder records or invent associations: it only
/// groups events already related through `parent_assistant_id` and preserves
/// the boundaries observed in the input sequence.
#[must_use]
pub fn project_to_rig_messages(
  logs: Vec<LogMessage>,
  options: &ProjectionOptions,
) -> Vec<Message> {
  let mut projected = Vec::new();
  let mut pending = None;

  for log in logs {
    let LogMessage { id, kind, .. } = log;
    match kind {
      LogMessageKind::User { content } => {
        flush_pending(&mut projected, &mut pending);
        projected.push(Message::User {
          content: vec![UserContent::Text(Text::new(content))],
        });
      }
      LogMessageKind::Assistant {
        content,
        provider,
        provider_response_id,
        ..
      } => {
        flush_pending(&mut projected, &mut pending);
        pending = Some(PendingAssistant::new(
          id,
          content,
          provider,
          provider_response_id,
        ));
      }
      LogMessageKind::ToolCall {
        tool_name,
        arguments,
        parent_assistant_id,
        ..
      } => {
        let Some(current) = pending.as_mut() else {
          continue;
        };
        if parent_assistant_id.as_deref() != Some(current.id.as_str()) {
          continue;
        }
        current.tool_calls.push(ToolCall::new(
          ToolCallId::new_or_mint(id),
          ToolFunction::new(tool_name, arguments),
        ));
      }
      LogMessageKind::Thinking {
        reasoning,
        parent_assistant_id,
      } => {
        let Some(current) = pending.as_mut() else {
          continue;
        };
        let allowed = include_thinking(options, current.provider.as_deref());
        if allowed
          && parent_assistant_id.as_deref() == Some(current.id.as_str())
        {
          current.reasoning.push(reasoning);
        }
      }
      LogMessageKind::ToolResult {
        tool_call_id,
        result,
        is_error,
        error_message,
      } => {
        flush_pending(&mut projected, &mut pending);
        let text = if is_error {
          serde_json::json!({
            "error": error_message.as_deref().unwrap_or("tool call failed")
          })
          .to_string()
        } else {
          result.to_string()
        };
        projected.push(Message::User {
          content: vec![UserContent::ToolResult(ToolResult {
            call: ToolCallId::new_or_mint(tool_call_id),
            provider: None,
            name: String::new(),
            content: vec![ToolResultContent::Text(Text::new(text))],
          })],
        });
      }
      LogMessageKind::Turn { .. }
      | LogMessageKind::Step { .. }
      | LogMessageKind::Condensation { .. } => {}
    }
  }

  flush_pending(&mut projected, &mut pending);
  projected
}

fn include_thinking(
  options: &ProjectionOptions,
  provider: Option<&str>,
) -> bool {
  match &options.thinking_policy {
    ThinkingPolicy::Never => false,
    ThinkingPolicy::Always => true,
    ThinkingPolicy::Conditional(should_include) => {
      should_include(provider.unwrap_or(""))
    }
  }
}

fn flush_pending(
  projected: &mut Vec<Message>,
  pending: &mut Option<PendingAssistant>,
) {
  let Some(pending) = pending.take() else {
    return;
  };
  projected.push(pending.into());
}

fn to_rig_reasoning(reasoning: StoredReasoning) -> Reasoning {
  Reasoning {
    id: reasoning.id,
    content: reasoning
      .content
      .into_iter()
      .map(to_rig_reasoning_content)
      .collect(),
  }
}

fn to_rig_reasoning_content(
  content: StoredReasoningContent,
) -> ReasoningContent {
  match content {
    StoredReasoningContent::Text { text, signature } => {
      ReasoningContent::Text { text, signature }
    }
    StoredReasoningContent::Encrypted(data) => {
      ReasoningContent::Encrypted(data)
    }
    StoredReasoningContent::Redacted { data } => {
      ReasoningContent::Redacted { data }
    }
    StoredReasoningContent::Summary(data) => ReasoningContent::Summary(data),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::memory::log::types::{AgentId, SessionId, now_millis};

  fn assistant(id: &str, provider: Option<&str>) -> LogMessage {
    message(
      id,
      LogMessageKind::Assistant {
        content: "answer".to_owned(),
        provider: provider.map(str::to_owned),
        model: None,
        provider_response_id: Some("response-1".to_owned()),
      },
    )
  }

  fn message(id: &str, kind: LogMessageKind) -> LogMessage {
    LogMessage {
      id: id.to_owned(),
      agent_id: AgentId::from("agent"),
      agent_nickname: "Agent".to_owned(),
      session_id: SessionId::from("session"),
      timestamp: now_millis(),
      kind,
    }
  }

  #[test]
  fn projects_assistant_tools_and_thinking_in_order() {
    let logs = vec![
      assistant("assistant", Some("provider")),
      message(
        "tool",
        LogMessageKind::ToolCall {
          tool_name: "lookup".to_owned(),
          arguments: serde_json::json!({"key": "value"}),
          parent_assistant_id: Some("assistant".to_owned()),
          step_id: None,
        },
      ),
      message(
        "thinking",
        LogMessageKind::Thinking {
          reasoning: StoredReasoning {
            id: Some("reasoning".to_owned()),
            content: vec![StoredReasoningContent::Summary(
              "summary".to_owned(),
            )],
          },
          parent_assistant_id: Some("assistant".to_owned()),
        },
      ),
    ];
    let options = ProjectionOptions {
      thinking_policy: ThinkingPolicy::Always,
    };

    let projected = project_to_rig_messages(logs, &options);

    let Some(Message::Assistant { content, .. }) = projected.first() else {
      return;
    };
    assert!(matches!(content.first(), Some(AssistantContent::Text(_))));
    assert!(matches!(
      content.get(1),
      Some(AssistantContent::ToolCall(_))
    ));
    assert!(matches!(
      content.get(2),
      Some(AssistantContent::Reasoning(_))
    ));
  }

  #[test]
  fn omits_orphan_tool_calls_and_projects_tool_errors_as_json() {
    let logs = vec![
      message(
        "orphan",
        LogMessageKind::ToolCall {
          tool_name: "lookup".to_owned(),
          arguments: serde_json::json!({}),
          parent_assistant_id: Some("missing".to_owned()),
          step_id: None,
        },
      ),
      message(
        "result",
        LogMessageKind::ToolResult {
          tool_call_id: "orphan".to_owned(),
          result: serde_json::Value::Null,
          is_error: true,
          error_message: Some("failed".to_owned()),
        },
      ),
    ];

    let projected =
      project_to_rig_messages(logs, &ProjectionOptions::default());

    assert_eq!(projected.len(), 1);
    let Some(Message::User { content }) = projected.first() else {
      return;
    };
    let Some(UserContent::ToolResult(result)) = content.first() else {
      return;
    };
    assert_eq!(
      result.content.first().and_then(|item| item.as_text()),
      Some(r#"{"error":"failed"}"#)
    );
  }

  #[test]
  fn ignores_tools_and_thinking_before_their_assistant() {
    let logs = vec![
      message(
        "tool-before-assistant",
        LogMessageKind::ToolCall {
          tool_name: "lookup".to_owned(),
          arguments: serde_json::json!({}),
          parent_assistant_id: Some("assistant".to_owned()),
          step_id: None,
        },
      ),
      message(
        "thinking-before-assistant",
        LogMessageKind::Thinking {
          reasoning: StoredReasoning {
            id: None,
            content: vec![StoredReasoningContent::Summary(
              "discarded".to_owned(),
            )],
          },
          parent_assistant_id: Some("assistant".to_owned()),
        },
      ),
      assistant("assistant", Some("provider")),
    ];

    let projected = project_to_rig_messages(
      logs,
      &ProjectionOptions {
        thinking_policy: ThinkingPolicy::Always,
      },
    );

    let Some(Message::Assistant { content, .. }) = projected.first() else {
      panic!("expected an assistant message");
    };
    assert_eq!(content.len(), 1);
    assert!(matches!(content.first(), Some(AssistantContent::Text(_))));
  }

  #[test]
  fn ignores_tool_calls_with_a_different_parent() {
    let logs = vec![
      assistant("assistant", Some("provider")),
      message(
        "wrong-parent",
        LogMessageKind::ToolCall {
          tool_name: "lookup".to_owned(),
          arguments: serde_json::json!({}),
          parent_assistant_id: Some("other-assistant".to_owned()),
          step_id: None,
        },
      ),
    ];

    let projected =
      project_to_rig_messages(logs, &ProjectionOptions::default());

    let Some(Message::Assistant { content, .. }) = projected.first() else {
      panic!("expected an assistant message");
    };
    assert_eq!(content.len(), 1);
    assert!(matches!(content.first(), Some(AssistantContent::Text(_))));
  }

  #[test]
  fn flushes_consecutive_assistants_and_user_boundaries() {
    let logs = vec![
      assistant("first", Some("provider")),
      assistant("second", Some("provider")),
      message(
        "user",
        LogMessageKind::User {
          content: "hello".to_owned(),
        },
      ),
    ];

    let projected =
      project_to_rig_messages(logs, &ProjectionOptions::default());

    assert_eq!(projected.len(), 3);
    assert!(matches!(projected.first(), Some(Message::Assistant { .. })));
    assert!(matches!(projected.get(1), Some(Message::Assistant { .. })));
    assert!(matches!(projected.get(2), Some(Message::User { .. })));
  }

  #[test]
  fn emits_tool_results_without_a_matching_tool_call() {
    let logs = vec![message(
      "result",
      LogMessageKind::ToolResult {
        tool_call_id: "missing-call".to_owned(),
        result: serde_json::json!({"ok": true}),
        is_error: false,
        error_message: None,
      },
    )];

    let projected =
      project_to_rig_messages(logs, &ProjectionOptions::default());

    assert_eq!(projected.len(), 1);
    assert!(matches!(projected.first(), Some(Message::User { .. })));
  }

  #[test]
  fn preserves_input_order_for_tool_result_before_assistant() {
    let logs = vec![
      message(
        "result",
        LogMessageKind::ToolResult {
          tool_call_id: "call".to_owned(),
          result: serde_json::json!("done"),
          is_error: false,
          error_message: None,
        },
      ),
      assistant("assistant", Some("provider")),
    ];

    let projected =
      project_to_rig_messages(logs, &ProjectionOptions::default());

    assert_eq!(projected.len(), 2);
    assert!(matches!(projected.first(), Some(Message::User { .. })));
    assert!(matches!(projected.get(1), Some(Message::Assistant { .. })));
  }

  #[test]
  fn does_not_attach_tool_calls_after_a_user_boundary() {
    let logs = vec![
      assistant("assistant", Some("provider")),
      message(
        "user",
        LogMessageKind::User {
          content: "hello".to_owned(),
        },
      ),
      message(
        "tool-after-user",
        LogMessageKind::ToolCall {
          tool_name: "lookup".to_owned(),
          arguments: serde_json::json!({}),
          parent_assistant_id: Some("assistant".to_owned()),
          step_id: None,
        },
      ),
    ];

    let projected =
      project_to_rig_messages(logs, &ProjectionOptions::default());

    assert_eq!(projected.len(), 2);
    let Some(Message::Assistant { content, .. }) = projected.first() else {
      panic!("expected an assistant message");
    };
    assert_eq!(content.len(), 1);
  }

  #[test]
  fn groups_interleaved_tools_and_thinking_by_content_type() {
    let logs = vec![
      assistant("assistant", Some("provider")),
      message(
        "thinking-1",
        LogMessageKind::Thinking {
          reasoning: StoredReasoning {
            id: None,
            content: vec![StoredReasoningContent::Summary("first".to_owned())],
          },
          parent_assistant_id: Some("assistant".to_owned()),
        },
      ),
      message(
        "tool-1",
        LogMessageKind::ToolCall {
          tool_name: "lookup".to_owned(),
          arguments: serde_json::json!({}),
          parent_assistant_id: Some("assistant".to_owned()),
          step_id: None,
        },
      ),
      message(
        "thinking-2",
        LogMessageKind::Thinking {
          reasoning: StoredReasoning {
            id: None,
            content: vec![StoredReasoningContent::Summary("second".to_owned())],
          },
          parent_assistant_id: Some("assistant".to_owned()),
        },
      ),
    ];

    let projected = project_to_rig_messages(
      logs,
      &ProjectionOptions {
        thinking_policy: ThinkingPolicy::Always,
      },
    );

    let Some(Message::Assistant { content, .. }) = projected.first() else {
      panic!("expected an assistant message");
    };
    assert!(matches!(
      content.get(1),
      Some(AssistantContent::ToolCall(_))
    ));
    assert!(matches!(
      content.get(2),
      Some(AssistantContent::Reasoning(_))
    ));
    assert!(matches!(
      content.get(3),
      Some(AssistantContent::Reasoning(_))
    ));
  }

  #[test]
  fn applies_thinking_policies() {
    let logs = vec![
      assistant("accepted", Some("accepted-provider")),
      message(
        "accepted-thinking",
        LogMessageKind::Thinking {
          reasoning: StoredReasoning {
            id: None,
            content: vec![StoredReasoningContent::Summary(
              "included".to_owned(),
            )],
          },
          parent_assistant_id: Some("accepted".to_owned()),
        },
      ),
      assistant("rejected", Some("rejected-provider")),
      message(
        "rejected-thinking",
        LogMessageKind::Thinking {
          reasoning: StoredReasoning {
            id: None,
            content: vec![StoredReasoningContent::Summary(
              "excluded".to_owned(),
            )],
          },
          parent_assistant_id: Some("rejected".to_owned()),
        },
      ),
    ];

    let projected = project_to_rig_messages(
      logs,
      &ProjectionOptions {
        thinking_policy: ThinkingPolicy::Conditional(Arc::new(|provider| {
          provider == "accepted-provider"
        })),
      },
    );

    assert_eq!(projected.len(), 2);
    let Some(Message::Assistant {
      content: accepted, ..
    }) = projected.first()
    else {
      panic!("expected the first message to be an assistant");
    };
    let Some(Message::Assistant {
      content: rejected, ..
    }) = projected.get(1)
    else {
      panic!("expected the second message to be an assistant");
    };
    assert_eq!(accepted.len(), 2);
    assert_eq!(rejected.len(), 1);
  }

  #[test]
  fn ignores_internal_events_and_supports_empty_history() {
    let logs = vec![
      message("turn", LogMessageKind::Turn { turn_number: 1 }),
      message(
        "step",
        LogMessageKind::Step {
          step_number: 1,
          turn_id: "turn".to_owned(),
        },
      ),
      message(
        "condensation",
        LogMessageKind::Condensation {
          summary: "summary".to_owned(),
          model_used: None,
        },
      ),
    ];

    assert!(
      project_to_rig_messages(Vec::new(), &ProjectionOptions::default())
        .is_empty()
    );
    assert!(
      project_to_rig_messages(logs, &ProjectionOptions::default()).is_empty()
    );
  }

  #[test]
  fn projects_all_reasoning_content_variants() {
    let logs = vec![
      assistant("assistant", Some("provider")),
      message(
        "thinking",
        LogMessageKind::Thinking {
          reasoning: StoredReasoning {
            id: Some("reasoning".to_owned()),
            content: vec![
              StoredReasoningContent::Text {
                text: "visible".to_owned(),
                signature: Some("signature".to_owned()),
              },
              StoredReasoningContent::Encrypted("encrypted".to_owned()),
              StoredReasoningContent::Redacted {
                data: "redacted".to_owned(),
              },
              StoredReasoningContent::Summary("summary".to_owned()),
            ],
          },
          parent_assistant_id: Some("assistant".to_owned()),
        },
      ),
    ];

    let projected = project_to_rig_messages(
      logs,
      &ProjectionOptions {
        thinking_policy: ThinkingPolicy::Always,
      },
    );

    let Some(Message::Assistant { content, .. }) = projected.first() else {
      panic!("expected an assistant message");
    };
    assert!(matches!(
      content.get(1),
      Some(AssistantContent::Reasoning(_))
    ));
  }
}
