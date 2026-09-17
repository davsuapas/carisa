//! Error values for the skill subsystem.
//!
//! These errors describe parsing issues coming from markdown-based skills and
//! lookups against the registry when a requested skill id is missing.

use thiserror::Error;

/// Error produced when parsing a skill from a markdown string.
///
/// All variants carry `id` and `title` context fields
/// (`Option<String>`) to enrich error messages when the
/// frontmatter has been partially parsed.
#[derive(Error, Debug, PartialEq)]
#[non_exhaustive]
pub enum MarkdownSkillError {
  /// The YAML frontmatter contains invalid syntax.
  YamlParse {
    /// Skill id, if parsed before the error occurred.
    id: Option<String>,
    /// Skill title, if parsed before the error occurred.
    title: Option<String>,
    /// Human-readable YAML parse error.
    msg: String,
  },
  /// A required field is missing from the frontmatter.
  MissingField {
    /// Skill id, if parsed.
    id: Option<String>,
    /// Skill title, if parsed.
    title: Option<String>,
    /// Name of the missing field (`"id"`, `"title"`, or `"description"`).
    field: String,
  },
  /// The markdown body after the frontmatter is empty or whitespace-only.
  EmptyBody {
    /// Skill id, if parsed.
    id: Option<String>,
    /// Skill title, if parsed.
    title: Option<String>,
  },
  /// The markdown input does not have the expected `---` delimiters
  /// around the YAML frontmatter.
  InvalidDelimiters {
    /// Skill id, if parsed.
    id: Option<String>,
    /// Skill title, if parsed.
    title: Option<String>,
  },
}

impl std::fmt::Display for MarkdownSkillError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      Self::YamlParse { id, title: _, msg } => {
        if let Some(id) = id {
          write!(
            f,
            "Failed to parse YAML frontmatter for skill \
                         '{id}': {msg}"
          )
        } else {
          write!(f, "Failed to parse YAML frontmatter: {msg}")
        }
      }
      Self::MissingField {
        id,
        title: _,
        field,
      } => {
        if let Some(id) = id {
          write!(f, "Missing required field '{field}' for skill '{id}'")
        } else {
          write!(f, "Missing required field '{field}'")
        }
      }
      Self::EmptyBody { id, title: _ } => {
        if let Some(id) = id {
          write!(f, "Skill '{id}' has an empty body")
        } else {
          write!(f, "Skill has an empty body")
        }
      }
      Self::InvalidDelimiters { id, title: _ } => {
        if let Some(id) = id {
          write!(f, "Invalid markdown delimiters for skill '{id}'")
        } else {
          write!(f, "Invalid markdown delimiters")
        }
      }
    }
  }
}

/// Error returned when a requested skill id is not found in the manager.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
#[error("Skill '{id}' not found")]
pub struct LoadSkillError {
  /// The id that was requested but not found.
  pub id: String,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn yaml_parse_with_id_includes_id_in_display() {
    let bad_yaml = "[unclosed_seq";
    let yaml_err =
      serde_saphyr::from_str::<serde::de::IgnoredAny>(bad_yaml).unwrap_err();
    let err = MarkdownSkillError::YamlParse {
      id: Some("x".to_owned()),
      title: Some("T".to_owned()),
      msg: yaml_err.to_string(),
    };
    let msg = err.to_string();
    assert!(
      msg.contains("skill 'x'"),
      "Display should contain skill id 'x', got: {msg}"
    );
  }

  #[test]
  fn missing_field_without_id_is_readable() {
    let err = MarkdownSkillError::MissingField {
      id: None,
      title: None,
      field: "description".to_owned(),
    };
    let msg = err.to_string();
    assert!(
      msg.contains("Missing required field 'description'"),
      "Display should mention the missing field, got: {msg}"
    );
    assert!(
      !msg.contains("for skill"),
      "Display should not mention 'for skill' when id is None, \
             got: {msg}"
    );
  }

  #[test]
  fn load_error_not_found_includes_id_in_display() {
    let err = LoadSkillError {
      id: "my-skill".to_owned(),
    };
    let msg = err.to_string();
    assert!(
      msg.contains("'my-skill'"),
      "Display should contain the skill id, got: {msg}"
    );
  }

  #[test]
  fn yaml_parse_without_id_is_readable() {
    let bad_yaml = "[unclosed_seq";
    let yaml_err =
      serde_saphyr::from_str::<serde::de::IgnoredAny>(bad_yaml).unwrap_err();
    let err = MarkdownSkillError::YamlParse {
      id: None,
      title: None,
      msg: yaml_err.to_string(),
    };
    let msg = err.to_string();
    assert!(
      msg.starts_with("Failed to parse YAML frontmatter:"),
      "Display should start with the error description, got: {msg}"
    );
  }

  #[test]
  fn empty_body_format() {
    let with_id = MarkdownSkillError::EmptyBody {
      id: Some("x".to_owned()),
      title: None,
    };
    assert!(with_id.to_string().contains("Skill 'x'"));

    let without_id = MarkdownSkillError::EmptyBody {
      id: None,
      title: None,
    };
    assert!(!without_id.to_string().contains("for skill"));
  }

  #[test]
  fn invalid_delimiters_format() {
    let err = MarkdownSkillError::InvalidDelimiters {
      id: Some("y".to_owned()),
      title: None,
    };
    assert!(err.to_string().contains("skill 'y'"));
  }

  #[test]
  fn markdown_error_implements_std_error() {
    fn assert_error<T: std::error::Error>() {}
    assert_error::<MarkdownSkillError>();
  }

  #[test]
  fn load_error_implements_std_error() {
    fn assert_error<T: std::error::Error>() {}
    assert_error::<LoadSkillError>();
  }
}
