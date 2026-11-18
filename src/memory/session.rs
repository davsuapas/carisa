//! Session logging for agent interactions and streamed completions.
//!
//! This module keeps a durable history of a conversation with a completion
//! model. Each record stores the rig message exactly as it was sent or
//! produced, including the provider items that let the same model replay the
//! turn unchanged, which keeps signed reasoning valid and the provider's
//! prefix cache warm.
//!
//! The module covers:
//!
//! - reading the active history as the messages the next request sends,
//! - keeping lightweight metadata about a session lifecycle,
//! - persisting a streamed completion once it finishes,
//! - marking the preamble and the condensation summary, which the storage
//!   backend uses to tell them apart and to split the history into
//!   partitions,
//! - archiving closed sessions to cold storage without duplicating data.
//!
//! A history read back is already provider-agnostic and needs no projection:
//! rig validates and shapes it for the target model when the request is sent.
//!
//! The usual flow is:
//!
//! 1. a live stream starts producing output,
//! 2. the stream is finished and folded into one assistant turn,
//! 3. durable messages are saved into the active session log,
//! 4. an inactive session can later be moved to archival storage.

pub mod error;
pub mod migration;
pub mod redis;
pub mod storage;
pub mod types;

pub mod inmemory;

pub use error::LogError;
pub use storage::{ColdStorage, LogStorage, SessionMetaStore};
pub use types::{
  AgentId, Etiqueta, LogMessage, MessageId, RIG_CORE_VERSION, SessionId,
  SessionMeta, StoredLogMessage, TimestampMillis,
};
