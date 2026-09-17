//! Errors shared by storage backends and log operations.

use thiserror::Error as ThisError;

/// Error produced while operating on a session log.
#[non_exhaustive]
#[derive(Debug, ThisError)]
pub enum LogError {
  /// Error returned by the Redis backend.
  #[error("Redis backend error: {message}")]
  Redis {
    /// Diagnostic message returned by the backend.
    message: String,
  },
  /// Error while serializing or deserializing JSON.
  #[error("JSON error: {message}")]
  Json {
    /// Diagnostic message returned by the JSON codec.
    message: String,
  },
  /// Error while compressing a log payload.
  #[error("compression error: {message}")]
  Compression {
    /// Diagnostic message returned by the compressor.
    message: String,
  },
  /// Error while decompressing a log payload.
  #[error("decompression error: {message}")]
  Decompression {
    /// Diagnostic message returned by the decompressor.
    message: String,
  },
  /// Error reported by the Rig completion stream.
  #[error(transparent)]
  Completion(#[from] rig_core::completion::CompletionError),
  /// Error joining a Tokio blocking task.
  #[error(transparent)]
  Task(#[from] tokio::task::JoinError),
  /// The requested cursor cannot be represented by the backend.
  #[error("batch index {batch_index} cannot be represented by the backend")]
  InvalidBatchIndex {
    /// Cursor value that could not be represented.
    batch_index: usize,
  },
  /// An invariant of an in-memory backend was violated.
  #[error("internal log storage error: {message}")]
  Internal {
    /// Description of the violated invariant.
    message: String,
  },
  /// An error reported by a storage backend during a log operation.
  #[error("log storage error: {message}")]
  Backend {
    /// Diagnostic message reported by the backend.
    message: String,
  },
}

impl From<redis::RedisError> for LogError {
  fn from(error: redis::RedisError) -> Self {
    Self::Redis {
      message: error.to_string(),
    }
  }
}

impl From<serde_json::Error> for LogError {
  fn from(error: serde_json::Error) -> Self {
    Self::Json {
      message: error.to_string(),
    }
  }
}
