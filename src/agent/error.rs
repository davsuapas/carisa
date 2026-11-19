//! Errors raised while validating and building an agent platform.
//!
//! Every failure carries the configuration `path` that produced it, so an
//! operator can point at the offending JSON key without reading a backtrace.
//! Secret values never appear in a message: a failure names the variable or
//! file, never the credential read from it.

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
  /// A secret names an environment variable that is not set.
  #[error("environment variable `{variable}` is not set")]
  MissingEnv {
    /// The variable named by the secret.
    variable: String,
  },
  /// A secret names a file that cannot be read.
  #[error("failed to read secret file `{path}`: {source}")]
  SecretFile {
    /// The file named by the secret.
    path: String,
    /// Underlying IO error.
    #[source]
    source: std::io::Error,
  },
  /// A secret resolved to an empty value.
  #[error("secret at `{path}` is empty")]
  EmptySecret {
    /// The secret source that produced nothing.
    path: String,
  },
  /// The configuration declares a schema version this build cannot read.
  #[error(
    "unsupported `schema_version` {version}: this build reads version 1 only"
  )]
  SchemaVersion {
    /// The version found in the configuration.
    version: u64,
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

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn messages_carry_the_path_and_never_a_value() {
    let error = PlatformError::invalid("providers.openai.kind", "unknown");
    assert_eq!(
      error.to_string(),
      "invalid value at `providers.openai.kind`: unknown"
    );

    let missing = PlatformError::MissingEnv {
      variable: "OPENAI_API_KEY".to_owned(),
    };
    assert_eq!(
      missing.to_string(),
      "environment variable `OPENAI_API_KEY` is not set"
    );
  }
}
