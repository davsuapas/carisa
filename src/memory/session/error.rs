//! Errors shared by storage backends and log operations.

use thiserror::Error as ThisError;

/// Error produced while operating on a session log.
#[non_exhaustive]
#[derive(Debug, ThisError)]
pub enum LogError {
  /// Error reported by a storage backend.
  #[error("storage backend error: {source}")]
  Storage {
    /// Original error returned by the backend.
    #[source]
    source: Box<dyn std::error::Error + Send + Sync>,
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
  Completion(Box<rig_core::error::ProviderError>),
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
}

impl From<rig_core::error::ProviderError> for LogError {
  fn from(error: rig_core::error::ProviderError) -> Self {
    Self::Completion(Box::new(error))
  }
}

impl From<serde_json::Error> for LogError {
  fn from(error: serde_json::Error) -> Self {
    Self::Json {
      message: error.to_string(),
    }
  }
}

#[cfg(test)]
mod tests {
  use std::error::Error;

  use super::LogError;

  #[test]
  fn storage_error_exposes_its_source() {
    let error = LogError::Storage {
      source: Box::new(std::io::Error::other("backend unavailable")),
    };

    assert_eq!(
      error.source().map(ToString::to_string).as_deref(),
      Some("backend unavailable"),
    );
  }
}
