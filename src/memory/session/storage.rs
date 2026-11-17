//! Asynchronous contracts for storing session events and metadata.

use async_trait::async_trait;
use rig_core::completion::Message;

use super::{
  error::LogError,
  types::{LogMessage, MessageId, SessionMeta, StoredLogMessage},
};

/// Active backend for a session's ordered history.
#[async_trait]
pub trait LogStorage: Send + Sync {
  /// Appends messages and returns their identifiers in the same order.
  async fn append(
    &self,
    agent_id: &str,
    session_id: &str,
    messages: Vec<LogMessage>,
  ) -> Result<Vec<MessageId>, LogError>;

  /// Reads the active message context as completion messages.
  ///
  /// The result is the conversation the next request sends, in order: it goes
  /// straight to a completion request, and the provider's request preparation
  /// validates and shapes it for the target model. No projection is applied
  /// here, because the stored records already hold rig's own messages.
  ///
  /// The context is kept small by condensation: only the messages of the
  /// current partition are returned.
  async fn history(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<Vec<Message>, LogError>;

  /// Reads the next batch of messages for a session, without exposing how the
  /// backend stores them internally.
  ///
  /// `batch_index` is an opaque cursor for the caller; the backend decides how
  /// to map that cursor to the next in-memory batch. Returning `None` means the
  /// session has no more batches to migrate.
  async fn drain_next_batch(
    &self,
    agent_id: &str,
    session_id: &str,
    batch_index: usize,
  ) -> Result<Option<(usize, Vec<StoredLogMessage>)>, LogError>;

  /// Deletes all messages from a session.
  async fn delete_session(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<(), LogError>;
}

/// Cold backend for retaining migrated messages idempotently.
#[async_trait]
pub trait ColdStorage: Send + Sync {
  /// Appends opaque message payloads without duplicating stored identifiers.
  ///
  /// The payload must be stored without decompression or recompression.
  /// Decoding belongs to the read path, not migration.
  async fn append(
    &self,
    agent_id: &str,
    session_id: &str,
    messages: Vec<StoredLogMessage>,
  ) -> Result<(), LogError>;

  /// Stores or replaces archived session metadata.
  async fn put_meta(&self, meta: &SessionMeta) -> Result<(), LogError>;

  /// Gets archived metadata if the session exists.
  async fn get_meta(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<Option<SessionMeta>, LogError>;
}

/// Independent backend for lightweight session metadata.
#[async_trait]
pub trait SessionMetaStore: Send + Sync {
  /// Stores or replaces a session's metadata.
  async fn put_meta(&self, meta: &SessionMeta) -> Result<(), LogError>;

  /// Gets metadata if the session exists.
  async fn get_meta(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<Option<SessionMeta>, LogError>;

  /// Deletes a session's metadata.
  async fn delete_meta(
    &self,
    agent_id: &str,
    session_id: &str,
  ) -> Result<(), LogError>;
}
