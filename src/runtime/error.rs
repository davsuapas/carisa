//! Errors raised while validating and building a runtime platform.

use thiserror::Error;

/// Validation and configuration errors for a runtime platform.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PlatformError {
  /// A value referenced by path is missing or invalid.
  #[error("invalid value at `{path}`: {message}")]
  Invalid {
    /// The configuration path that failed validation.
    path: String,
    /// Human-readable explanation.
    message: String,
  },
  /// Failed to read or parse a configuration file.
  #[error("failed to read `{path}`: {source}")]
  Io {
    /// The file or field path being processed.
    path: String,
    /// Underlying IO error.
    #[source]
    source: std::io::Error,
  },
  /// Failed to deserialize JSON or configuration input.
  #[error("failed to parse `{path}`: {source}")]
  Json {
    /// The config path that failed parsing.
    path: String,
    /// Underlying parse error.
    #[source]
    source: serde_json::Error,
  },
  /// Invalid base URL value.
  #[error("invalid URL at `{path}`: {message}")]
  Url {
    /// The path containing the invalid URL.
    path: String,
    /// Validation message.
    message: String,
  },
}

impl PlatformError {
  /// Creates an invalid-value error with a path and message.
  pub fn invalid(path: impl Into<String>, message: impl Into<String>) -> Self {
    Self::Invalid {
      path: path.into(),
      message: message.into(),
    }
  }
}
