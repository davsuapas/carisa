//! Serializable types that form a session log.
//!
//! A session log is the sequence of messages a completion model receives and
//! produces. Each record stores a [`rig_core::completion::Message`] **exactly**
//! as it was sent or produced, provider items included: those items are what
//! let the same model replay a turn unchanged, which keeps signed reasoning
//! valid and the provider's prefix cache warm. Stripping them would make rig
//! rebuild the turn from its canonical fields, changing the request prefix.
//!
//! [`LogMessage`] adds the identity a storage backend needs (a ULID, the
//! owning agent and session, a timestamp) and an [`Etiqueta`] that marks the
//! two records the session machinery writes on its own: the preamble and the
//! condensation summary. Everything else is part of the conversation.
//!
//! Reading a session returns the stored messages as they are; converting them
//! to a request ([`rig_core::completion::CompletionRequest`]) lets rig validate
//! and shape them for the target model. No projection of our own is needed.

use std::time::{SystemTime, UNIX_EPOCH};

use rig_core::completion::Message;
use serde::{Deserialize, Serialize};

pub use crate::identity::{AgentId, SessionId};

/// Minor version of `rig-core` whose message format this build stores.
///
/// The serialized shape of [`Message`] is not part of rig's stability
/// promise: variants get renamed or removed between minors. A session log
/// records the version it was written with so a reader can tell whether the
/// stored messages are still readable by this build. Bump this value together
/// with the `rig-core` dependency in `Cargo.toml`.
pub const RIG_CORE_VERSION: &str = "0.46";

/// Unique identifier of a persisted message.
pub type MessageId = String;
/// Timestamp expressed as milliseconds since the Unix epoch.
pub type TimestampMillis = i64;

/// Message in the representation used by a storage backend.
///
/// The payload is intentionally opaque to migration. A cold backend must
/// preserve it without decoding or recompressing it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct StoredLogMessage {
  /// Message identifier, kept outside the payload for indexing and deduplication.
  pub id: MessageId,
  /// Serialized message bytes in the backend's native format.
  pub payload: Vec<u8>,
}

/// Label for the records the session machinery writes on its own.
///
/// It is stored apart from the message so a backend can decide where a
/// partition starts without decoding the payload.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Etiqueta {
  /// Message that belongs to the conversation itself.
  None,
  /// System prompt the agent sends ahead of the conversation.
  ///
  /// Stored once, as a [`Message::System`], so a history read back starts
  /// with it.
  Preamble,
  /// Summary that condenses the history before it.
  ///
  /// Stored as a [`Message::User`] so no model wire moves or joins it, and
  /// used by the storage backend to open a new partition.
  Condensation,
}

/// Message stored in a session log.
///
/// The payload is the rig message exactly as it was sent or produced,
/// including the provider items that a model needs to replay its own turns
/// unchanged.
#[non_exhaustive]
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LogMessage {
  /// ULID of the message.
  pub id: MessageId,
  /// Agent that owns the message.
  pub agent_id: AgentId,
  /// Agent display name when the message was recorded.
  pub agent_nickname: String,
  /// Session that owns the message.
  pub session_id: SessionId,
  /// Message creation time.
  pub timestamp: TimestampMillis,
  /// Whether the record belongs to the conversation or was written by the
  /// session machinery.
  pub etiqueta: Etiqueta,
  /// Message sent to, or produced by, the completion model.
  pub message: Message,
}

impl LogMessage {
  /// A conversation message, labeled [`Etiqueta::None`].
  #[must_use]
  pub fn new(agent_id: &str, session_id: &str, message: Message) -> Self {
    Self::labeled(agent_id, session_id, Etiqueta::None, message)
  }

  /// The agent's preamble, stored once as a system message.
  #[must_use]
  pub fn preamble(
    agent_id: &str,
    session_id: &str,
    content: impl Into<String>,
  ) -> Self {
    Self::labeled(
      agent_id,
      session_id,
      Etiqueta::Preamble,
      Message::system(content),
    )
  }

  /// Stores an existing Rig system message as the session preamble.
  #[must_use]
  pub fn preamble_message(
    agent_id: &str,
    session_id: &str,
    message: Message,
  ) -> Self {
    Self::labeled(agent_id, session_id, Etiqueta::Preamble, message)
  }

  /// A condensation summary, stored as a user message.
  ///
  /// The storage backend opens a new partition with it.
  #[must_use]
  pub fn condensation(
    agent_id: &str,
    session_id: &str,
    summary: impl Into<String>,
  ) -> Self {
    Self::labeled(
      agent_id,
      session_id,
      Etiqueta::Condensation,
      Message::user(summary),
    )
  }

  /// Sets the agent display name recorded with the message.
  #[must_use]
  pub fn with_agent_nickname(mut self, nickname: impl Into<String>) -> Self {
    self.agent_nickname = nickname.into();
    self
  }

  fn labeled(
    agent_id: &str,
    session_id: &str,
    etiqueta: Etiqueta,
    message: Message,
  ) -> Self {
    Self {
      id: generate_message_id(),
      agent_id: agent_id.to_owned(),
      agent_nickname: String::new(),
      session_id: session_id.to_owned(),
      timestamp: now_millis(),
      etiqueta,
      message,
    }
  }

  /// A message with the given identity, for deterministic tests.
  #[cfg(test)]
  pub(crate) fn with_id(
    id: impl Into<MessageId>,
    agent_id: &str,
    agent_nickname: &str,
    session_id: &str,
    timestamp: TimestampMillis,
    etiqueta: Etiqueta,
    message: Message,
  ) -> Self {
    Self {
      id: id.into(),
      agent_id: agent_id.to_owned(),
      agent_nickname: agent_nickname.to_owned(),
      session_id: session_id.to_owned(),
      timestamp,
      etiqueta,
      message,
    }
  }
}

/// Persistent metadata for a session.
#[non_exhaustive]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionMeta {
  /// Owning agent.
  pub agent_id: AgentId,
  /// Agent display name.
  pub agent_nickname: String,
  /// Session described by this metadata.
  pub session_id: SessionId,
  /// Creation time.
  pub created_at: TimestampMillis,
  /// Closing time, if the session was closed.
  pub closed_at: Option<TimestampMillis>,
  /// External workflow identifier, if any.
  pub workflow_id: Option<String>,
  /// Minor version of `rig-core` the session log was written with.
  ///
  /// Compare it with [`RIG_CORE_VERSION`] before replaying a stored history:
  /// a log written by another minor may not deserialize with this build.
  /// Session records written before this field existed read as empty.
  #[serde(default)]
  pub rig_version: String,
}

impl SessionMeta {
  /// Metadata for a new session, stamped with the running `rig-core`
  /// version.
  #[must_use]
  pub fn new(agent_id: &str, agent_nickname: &str, session_id: &str) -> Self {
    Self {
      agent_id: agent_id.to_owned(),
      agent_nickname: agent_nickname.to_owned(),
      session_id: session_id.to_owned(),
      created_at: now_millis(),
      closed_at: None,
      workflow_id: None,
      rig_version: RIG_CORE_VERSION.to_owned(),
    }
  }
}

/// Generates a ULID for a new message.
#[must_use]
pub fn generate_message_id() -> MessageId {
  ulid::Ulid::generate().to_string()
}

/// Returns the current time as milliseconds since the Unix epoch.
///
/// Returns zero if the system clock is before the epoch.
#[must_use]
pub fn now_millis() -> TimestampMillis {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map_or(0, |duration| {
      i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
    })
}

#[cfg(test)]
mod tests {
  use rig_core::completion::message::{AssistantContent, AssistantMessage};
  use rstest::rstest;

  use super::*;

  fn provider_item() -> serde_json::Value {
    serde_json::json!({"type": "text", "text": "hello", "provider": "item"})
  }

  fn assistant_with_provider_item() -> Message {
    let text = AssistantContent::text("hello").with_native(provider_item());
    Message::Assistant(AssistantMessage::new(vec![text]))
  }

  #[test]
  fn log_message_round_trips_provider_items() {
    let log =
      LogMessage::new("agent", "session", assistant_with_provider_item());

    let serialized = serde_json::to_string(&log).expect("serialize");
    let restored: LogMessage =
      serde_json::from_str(&serialized).expect("deserialize");

    let Message::Assistant(turn) = &restored.message else {
      panic!("expected an assistant message");
    };
    let block = turn.content.first().expect("one block should exist");
    assert_eq!(block.native_item(), Some(&provider_item()));
  }

  #[test]
  fn condensation_is_a_user_message() {
    let log = LogMessage::condensation("agent", "session", "summary");

    assert_eq!(log.etiqueta, Etiqueta::Condensation);
    assert!(matches!(log.message, Message::User { .. }));
  }

  #[test]
  fn preamble_is_a_system_message() {
    let log = LogMessage::preamble("agent", "session", "system prompt");

    assert_eq!(log.etiqueta, Etiqueta::Preamble);
    assert!(matches!(log.message, Message::System { .. }));
  }

  #[test]
  fn preamble_message_preserves_rig_message_and_identity() {
    let message = Message::system("system prompt");
    let expected = serde_json::to_value(&message).expect("message serializes");
    let log = LogMessage::preamble_message("agent", "session", message);

    assert_eq!(log.agent_id, "agent");
    assert_eq!(log.session_id, "session");
    assert_eq!(log.etiqueta, Etiqueta::Preamble);
    assert_eq!(
      serde_json::to_value(&log.message).expect("message serializes"),
      expected
    );
  }

  #[test]
  fn new_stamps_identity_and_etiqueta() {
    let log = LogMessage::new("agent", "session", Message::user("hi"));

    assert_eq!(log.agent_id, "agent");
    assert_eq!(log.session_id, "session");
    assert_eq!(log.etiqueta, Etiqueta::None);
    assert!(log.timestamp > 0);
    assert_eq!(log.id.len(), 26);
  }

  #[test]
  fn new_session_meta_records_the_rig_version() {
    let meta = SessionMeta::new("agent", "Agent", "session");

    assert_eq!(meta.rig_version, RIG_CORE_VERSION);
    assert_eq!(meta.closed_at, None);
  }

  #[rstest]
  #[case::stored_version(
    r#"{"agent_id":"agent","agent_nickname":"Agent","session_id":"session",
    "created_at":1,"closed_at":null,"workflow_id":null,
    "rig_version":"0.46"}"#,
    "0.46"
  )]
  #[case::legacy_without_version(
    r#"{"agent_id":"agent","agent_nickname":"Agent","session_id":"session",
    "created_at":1,"closed_at":null,"workflow_id":null}"#,
    ""
  )]
  fn session_meta_reads_stored_and_legacy_json(
    #[case] json: &str,
    #[case] expected_version: &str,
  ) {
    let meta: SessionMeta = serde_json::from_str(json).expect("deserialize");

    assert_eq!(meta.rig_version, expected_version);
  }
}
