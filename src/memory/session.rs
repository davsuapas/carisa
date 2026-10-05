//! Session logging for agent interactions and streamed completions.
//!
//! This module keeps a durable history of what happened during a session: user
//! input, assistant output, intermediate reasoning, tool usage, and final
//! responses. It is designed for live agent execution, where output often arrives
//! incrementally and must be preserved in a structured way.
//!
//! The logging pipeline supports several important responsibilities:
//!
//! - collecting the active session history while it is still in use,
//! - keeping lightweight metadata about a session lifecycle,
//! - capturing streamed model output as it arrives,
//! - preserving the relationship between assistant responses, reasoning, and
//!   tool calls,
//! - projecting the stored history back into a richer message shape when the
//!   runtime needs to reconstruct the conversation,
//! - archiving closed sessions to cold storage without duplicating data.
//!
//! This is especially important in streaming scenarios, where text is usually
//! produced in chunks instead of a single final payload. The system accumulates
//! those chunks, forwards live updates to callbacks when needed, and stores the
//! completed sequence once the stream finishes.
//!
//! A few edge cases are handled explicitly:
//!
//! - partial text deltas are merged into one complete assistant answer,
//! - reasoning may appear in intermediate updates before it is finalized,
//! - tool calls must remain attached to the correct assistant response,
//! - transient or incomplete events are ignored instead of being persisted as
//!   final records,
//! - archival is idempotent, so retries remain safe and do not duplicate data.
//!
//! The usual flow is:
//!
//! 1. a live stream starts producing output,
//! 2. text and reasoning are surfaced in real time if requested,
//! 3. durable events are saved into the active session log,
//! 4. the final response is written once the stream completes,
//! 5. an inactive session can later be moved to archival storage.

pub mod error;
pub mod migration;
pub mod projection;
pub mod redis;
pub mod storage;
pub mod streaming;
pub mod types;

pub mod inmemory;

pub use error::LogError;
pub use storage::{ColdStorage, LogStorage, SessionMetaStore};
pub use types::{
  AgentId, LogMessage, LogMessageKind, MessageId, SessionId, SessionMeta,
  StoredLogMessage, StoredReasoning, StoredReasoningContent, TimestampMillis,
};
