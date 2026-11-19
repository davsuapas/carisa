//! Secret references accepted by agent platform configuration.
//!
//! JSON uses an externally tagged object with exactly one variant, for
//! example `{"literal":"value"}`, `{"env":"API_KEY"}`, or
//! `{"file":"/run/secrets/api-key"}`.
//!
//! These values record how a secret is described. [`Secret::resolve`] reads
//! the referenced variable or file and hands the value to `rig-core` as its
//! redacting credential type, which is the only place the plain value
//! travels. `Display` redacts all variants, but `Debug` does not, so
//! configuration values should never be logged wholesale.

use std::fmt;

use rig_core::wire::Secret as RigSecret;

use super::error::PlatformError;

/// Secret value that may be supplied from a literal, env var, or file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Secret {
  /// Literal secret value embedded directly in config.
  Literal(String),
  /// Secret loaded from an environment variable.
  Env(String),
  /// Secret loaded from a file path.
  File(String),
}

impl Secret {
  /// Reads the referenced value and returns it as a rig credential.
  ///
  /// An [`Env`](Self::Env) secret reads the process environment; a
  /// [`File`](Self::File) secret reads the file and trims surrounding
  /// whitespace, which is how credentials written with a trailing newline
  /// are meant to be read. A [`Literal`](Self::Literal) is used as written.
  ///
  /// # Errors
  ///
  /// [`PlatformError::MissingEnv`] when the variable is not set,
  /// [`PlatformError::SecretFile`] when the file cannot be read, and
  /// [`PlatformError::EmptySecret`] when the value is empty. No message
  /// includes the value itself.
  pub fn resolve(&self) -> Result<RigSecret, PlatformError> {
    match self {
      Self::Literal(value) => non_empty("literal", value, false),
      Self::Env(variable) => {
        let value =
          std::env::var(variable).map_err(|_| PlatformError::MissingEnv {
            variable: variable.clone(),
          })?;
        non_empty(&format!("env:{variable}"), &value, false)
      }
      Self::File(path) => {
        let value = std::fs::read_to_string(path).map_err(|source| {
          PlatformError::SecretFile {
            path: path.clone(),
            source,
          }
        })?;
        non_empty(&format!("file:{path}"), &value, true)
      }
    }
  }
}

/// Turns a resolved value into a credential, rejecting empty input.
fn non_empty(
  path: &str,
  value: &str,
  trim: bool,
) -> Result<RigSecret, PlatformError> {
  let value = if trim { value.trim() } else { value };
  if value.trim().is_empty() {
    return Err(PlatformError::EmptySecret {
      path: path.to_owned(),
    });
  }
  Ok(RigSecret::from(value.to_owned()))
}

impl fmt::Display for Secret {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "***")
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn secret_redacts_literal() {
    let secret = Secret::Literal("api-key".to_owned());
    assert_eq!(format!("{secret}"), "***");
  }

  #[test]
  fn literal_resolves_to_its_value() {
    let secret = Secret::Literal("api-key".to_owned());

    assert_eq!(
      secret.resolve().expect("literal resolves").expose(),
      "api-key"
    );
  }

  #[test]
  fn empty_literal_is_rejected() {
    let error = Secret::Literal("  ".to_owned())
      .resolve()
      .expect_err("a blank secret is not a credential");

    assert!(error.to_string().contains("literal"));
  }

  #[test]
  fn missing_environment_variable_names_the_variable_only() {
    let error = Secret::Env("CARISA_ABSENT_SECRET_FOR_TESTS".to_owned())
      .resolve()
      .expect_err("the variable is not set");

    assert_eq!(
      error.to_string(),
      "environment variable `CARISA_ABSENT_SECRET_FOR_TESTS` is not set"
    );
  }

  #[test]
  fn file_secret_is_trimmed() {
    let path = std::env::temp_dir()
      .join(format!("carisa-secret-{}.txt", ulid::Ulid::generate()));
    std::fs::write(&path, "file-key\n").expect("secret file is written");

    let resolved = Secret::File(path.to_string_lossy().to_string()).resolve();
    let removed = std::fs::remove_file(&path);

    assert_eq!(resolved.expect("file resolves").expose(), "file-key");
    removed.expect("secret file is removed");
  }

  #[test]
  fn unreadable_and_empty_files_are_errors() {
    let missing = Secret::File("/nonexistent/carisa-secret".to_owned())
      .resolve()
      .expect_err("the file does not exist");
    let path = std::env::temp_dir()
      .join(format!("carisa-secret-{}.txt", ulid::Ulid::generate()));
    std::fs::write(&path, "   \n").expect("secret file is written");
    let empty = Secret::File(path.to_string_lossy().to_string()).resolve();
    let removed = std::fs::remove_file(&path);

    assert!(missing.to_string().contains("/nonexistent/carisa-secret"));
    assert!(
      empty
        .expect_err("a blank file is not a credential")
        .to_string()
        .contains("is empty")
    );
    removed.expect("secret file is removed");
  }
}
