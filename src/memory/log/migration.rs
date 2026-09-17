//! Idempotent migration of sessions to cold storage.

use std::sync::Arc;

use super::{
  error::LogError,
  storage::{ColdStorage, LogStorage, SessionMetaStore},
};

/// Moves a closed session to cold storage and cleans the active backend.
///
/// Every message of the session is migrated, not just the active context:
/// messages are drained in batches and appended one batch at a time, so the
/// move stays idempotent and never requires loading the whole session in
/// memory at once.
///
/// # Errors
///
/// Returns the first error produced by a backend during migration.
pub async fn migrate_session_to_cold(
  agent_id: &str,
  session_id: &str,
  active_store: Arc<dyn LogStorage>,
  cold_store: Arc<dyn ColdStorage>,
  meta_store: Arc<dyn SessionMetaStore>,
) -> Result<(), LogError> {
  let Some(meta) = meta_store.get_meta(agent_id, session_id).await? else {
    return Ok(());
  };
  let mut batch_index = 0;
  loop {
    let Some((next_batch_index, batch)) = active_store
      .drain_next_batch(agent_id, session_id, batch_index)
      .await?
    else {
      break;
    };
    cold_store.append(agent_id, session_id, batch).await?;
    batch_index = next_batch_index;
  }

  cold_store.put_meta(&meta).await?;

  active_store.delete_session(agent_id, session_id).await?;
  meta_store.delete_meta(agent_id, session_id).await?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use std::sync::Arc;

  use async_trait::async_trait;

  use super::{LogError, migrate_session_to_cold};
  use crate::memory::log::{
    inmemory::{
      InMemoryColdStorage, InMemoryLogStorage, InMemorySessionMetaStore,
    },
    storage::{ColdStorage, LogStorage, SessionMetaStore},
    types::{LogMessage, LogMessageKind, SessionMeta, StoredLogMessage},
  };

  fn message(id: &str) -> LogMessage {
    LogMessage {
      id: id.to_owned(),
      agent_id: "agent".to_owned(),
      agent_nickname: "Agent".to_owned(),
      session_id: "session".to_owned(),
      timestamp: 1,
      kind: LogMessageKind::User {
        content: "hello".to_owned(),
      },
    }
  }

  fn condensation(id: &str) -> LogMessage {
    LogMessage {
      kind: LogMessageKind::Condensation {
        summary: "condensed".to_owned(),
        model_used: None,
      },
      ..message(id)
    }
  }

  fn meta() -> SessionMeta {
    SessionMeta {
      agent_id: "agent".to_owned(),
      agent_nickname: "Agent".to_owned(),
      session_id: "session".to_owned(),
      created_at: 1,
      closed_at: Some(2),
      workflow_id: None,
    }
  }

  fn log_store(storage: Arc<InMemoryLogStorage>) -> Arc<dyn LogStorage> {
    storage
  }

  fn cold_store(storage: Arc<InMemoryColdStorage>) -> Arc<dyn ColdStorage> {
    storage
  }

  fn metadata_store(
    storage: Arc<InMemorySessionMetaStore>,
  ) -> Arc<dyn SessionMetaStore> {
    storage
  }

  #[tokio::test]
  async fn migrates_and_is_idempotent() {
    let active = Arc::new(InMemoryLogStorage::new());
    let cold = Arc::new(InMemoryColdStorage::new());
    let metadata = Arc::new(InMemorySessionMetaStore::new());
    active
      .append("agent", "session", vec![message("one")])
      .await
      .expect("active append should work");
    metadata
      .put_meta(&meta())
      .await
      .expect("metadata should work");
    let active_store = log_store(Arc::clone(&active));
    let cold_store = cold_store(Arc::clone(&cold));
    let metadata_store = metadata_store(Arc::clone(&metadata));

    migrate_session_to_cold(
      "agent",
      "session",
      Arc::clone(&active_store),
      Arc::clone(&cold_store),
      Arc::clone(&metadata_store),
    )
    .await
    .expect("migration should work");
    migrate_session_to_cold(
      "agent",
      "session",
      active_store,
      cold_store,
      metadata_store,
    )
    .await
    .expect("second migration should work");

    assert!(
      active
        .history("agent", "session")
        .await
        .expect("history should work")
        .is_empty()
    );
    assert_eq!(
      cold
        .messages("agent", "session")
        .expect("cold read should work")
        .len(),
      1
    );
    assert_eq!(
      cold
        .get_meta("agent", "session")
        .await
        .expect("cold meta should work"),
      Some(meta())
    );
    assert!(
      metadata
        .get_meta("agent", "session")
        .await
        .expect("active meta should work")
        .is_none()
    );
  }

  #[tokio::test]
  async fn migrates_messages_from_every_partition() {
    let active = Arc::new(InMemoryLogStorage::new());
    let cold = Arc::new(InMemoryColdStorage::new());
    let metadata = Arc::new(InMemorySessionMetaStore::new());
    active
      .append(
        "agent",
        "session",
        vec![message("one"), condensation("summary"), message("two")],
      )
      .await
      .expect("active append should work");
    metadata
      .put_meta(&meta())
      .await
      .expect("metadata should work");

    migrate_session_to_cold(
      "agent",
      "session",
      log_store(Arc::clone(&active)),
      cold_store(Arc::clone(&cold)),
      metadata_store(Arc::clone(&metadata)),
    )
    .await
    .expect("migration should work");

    assert_eq!(
      cold
        .messages("agent", "session")
        .expect("cold read should work")
        .len(),
      3
    );
  }

  #[derive(Debug)]
  struct FailingColdStorage;

  #[async_trait]
  impl ColdStorage for FailingColdStorage {
    async fn append(
      &self,
      _: &str,
      _: &str,
      _: Vec<StoredLogMessage>,
    ) -> Result<(), LogError> {
      Err(LogError::Backend {
        message: "cold backend failed".to_owned(),
      })
    }

    async fn put_meta(&self, _: &SessionMeta) -> Result<(), LogError> {
      Ok(())
    }

    async fn get_meta(
      &self,
      _: &str,
      _: &str,
    ) -> Result<Option<SessionMeta>, LogError> {
      Ok(None)
    }
  }

  #[tokio::test]
  async fn keeps_active_messages_when_cold_storage_fails() {
    let active = Arc::new(InMemoryLogStorage::new());
    let metadata = Arc::new(InMemorySessionMetaStore::new());
    active
      .append("agent", "session", vec![message("one")])
      .await
      .expect("active append should work");
    metadata
      .put_meta(&meta())
      .await
      .expect("metadata should work");

    let result = migrate_session_to_cold(
      "agent",
      "session",
      log_store(Arc::clone(&active)),
      Arc::new(FailingColdStorage),
      metadata,
    )
    .await;

    assert!(result.is_err());
    assert_eq!(
      active
        .history("agent", "session")
        .await
        .expect("history should work")
        .len(),
      1
    );
  }
}
