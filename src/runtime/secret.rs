//! Secret references accepted by runtime configuration.
//!
//! JSON uses an externally tagged object with exactly one variant, for
//! example `{"literal":"value"}`, `{"env":"API_KEY"}`, or
//! `{"file":"/run/secrets/api-key"}`. In `CARISA_` variables, represent
//! the same variants with nested path components such as
//! `CARISA_PROVIDERS__OPENAI__API_KEY__ENV=API_KEY`.
//!
//! These values record how a secret is described; `AgentPlatformBuilder`
//! does not read the named environment variable or file. `Display` redacts
//! all variants, but `Debug` and literal configuration values should not be
//! treated as secret-safe output.

use std::fmt;

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
}
