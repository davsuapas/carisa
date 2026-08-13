//! Markdown parsing for domain skills.
//!
//! Skills defined in markdown are expected to provide YAML frontmatter with the
//! core metadata and a body containing the instructions. The parser validates
//! the required fields and rejects empty instruction bodies.

use crate::skill::error::MarkdownError;
use crate::skill::types::DomainSkill;

/// Best-effort extraction of a scalar YAML field for error context.
///
/// This is NOT a YAML parser. It simply scans lines for
/// `{field}: value` patterns.
#[expect(clippy::arithmetic_side_effects)]
fn quick_extract_field(yaml: &str, field: &str) -> Option<String> {
  for line in yaml.lines() {
    let trimmed = line.trim();
    let prefix = format!("{field}:");
    if let Some(rest) = trimmed.strip_prefix(&prefix) {
      let value = rest.trim();
      if value.is_empty() {
        return None;
      }
      if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        return Some(value[1..value.len() - 1].to_owned());
      }
      if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
        return Some(value[1..value.len() - 1].to_owned());
      }
      let without_comment = value.split('#').next().unwrap_or(value).trim();
      if without_comment.is_empty() {
        return None;
      }
      return Some(without_comment.to_owned());
    }
  }
  None
}

/// Split markdown content into (frontmatter, body).
///
/// Returns an error if the delimiters are missing or malformed.
#[expect(clippy::arithmetic_side_effects)]
fn split_frontmatter(content: &str) -> Result<(&str, &str), MarkdownError> {
  let rest = if let Some(r) = content.strip_prefix("---\n") {
    r
  } else if let Some(r) = content.strip_prefix("---\r\n") {
    r
  } else {
    return Err(MarkdownError::InvalidDelimiters {
      id: None,
      title: None,
    });
  };

  if let Some(pos) = rest.find("\n---\n") {
    let frontmatter = &rest[..pos];
    let body = &rest[pos + 5..];
    return Ok((frontmatter, body));
  }

  if let Some(pos) = rest.find("\n---\r\n") {
    let frontmatter = &rest[..pos];
    let body = &rest[pos + 6..];
    return Ok((frontmatter, body));
  }

  if rest.ends_with("\n---") {
    let end = rest.len() - 4;
    let frontmatter = &rest[..end];
    return Ok((frontmatter, ""));
  }

  Err(MarkdownError::InvalidDelimiters {
    id: None,
    title: None,
  })
}

/// Extract a string field from a YAML frontmatter string.
///
/// Returns `None` if the field is absent, empty, or is a non-string YAML
/// type (number, boolean, null).
#[expect(clippy::arithmetic_side_effects)]
fn extract_field(yaml: &str, field: &str) -> Option<String> {
  for line in yaml.lines() {
    let trimmed = line.trim();
    let prefix = format!("{field}:");
    if let Some(rest) = trimmed.strip_prefix(&prefix) {
      let value = rest.trim();
      if value.is_empty() {
        return None;
      }
      // Double-quoted string
      if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        return Some(value[1..value.len() - 1].to_owned());
      }
      // Single-quoted string
      if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
        return Some(value[1..value.len() - 1].to_owned());
      }
      // YAML boolean
      if matches!(
        value.to_lowercase().as_str(),
        "true" | "false" | "yes" | "no" | "on" | "off"
      ) {
        return None;
      }
      // YAML null
      if matches!(value.to_lowercase().as_str(), "~" | "null") {
        return None;
      }
      // YAML number
      if field != "version"
        && (value.parse::<i64>().is_ok() || value.parse::<f64>().is_ok())
      {
        return None;
      }
      // Inline comment
      let without_comment = value.split('#').next().unwrap_or(value).trim();
      if without_comment.is_empty() {
        return None;
      }
      return Some(without_comment.to_owned());
    }
  }
  None
}

impl DomainSkill {
  /// Parse a [`DomainSkill`] from a markdown string with YAML frontmatter.
  ///
  /// The expected format is:
  ///
  /// ```text
  /// ---
  /// id: my-skill
  /// title: My Skill
  /// description: Does useful things
  /// version: 1.0.0
  /// ---
  /// Step 1: initialize
  /// Step 2: execute
  /// ```
  ///
  /// The frontmatter must be valid YAML and contain non-empty string
  /// values for `id`, `title`, and `description`. The body (instructions)
  /// must be non-empty after trimming whitespace.
  ///
  /// # Errors
  ///
  /// Returns [`MarkdownError::InvalidDelimiters`] if the opening `---\n`
  /// or closing `\n---\n` delimiters are missing or malformed.
  ///
  /// Returns [`MarkdownError::YamlParse`] if the frontmatter is not valid
  /// YAML.
  ///
  /// Returns [`MarkdownError::MissingField`] if `id`, `title`,
  /// `description`, or `version` is missing or is not a YAML string.
  ///
  /// Returns [`MarkdownError::EmptyBody`] if the body is empty or
  /// whitespace-only after trimming.
  pub fn from_markdown(content: &str) -> Result<Self, MarkdownError> {
    let (frontmatter, body) = split_frontmatter(content)?;

    let scanned_id = quick_extract_field(frontmatter, "id");
    let scanned_title = quick_extract_field(frontmatter, "title");

    serde_saphyr::from_str::<serde::de::IgnoredAny>(frontmatter).map_err(
      |e| MarkdownError::YamlParse {
        id: scanned_id.clone(),
        title: scanned_title.clone(),
        msg: e.to_string(),
      },
    )?;

    let id = extract_field(frontmatter, "id").ok_or_else(|| {
      MarkdownError::MissingField {
        id: scanned_id,
        title: scanned_title,
        field: "id".to_owned(),
      }
    })?;

    let title = extract_field(frontmatter, "title").ok_or_else(|| {
      MarkdownError::MissingField {
        id: Some(id.clone()),
        title: None,
        field: "title".to_owned(),
      }
    })?;

    let description =
      extract_field(frontmatter, "description").ok_or_else(|| {
        MarkdownError::MissingField {
          id: Some(id.clone()),
          title: Some(title.clone()),
          field: "description".to_owned(),
        }
      })?;

    let version = extract_field(frontmatter, "version").ok_or_else(|| {
      MarkdownError::MissingField {
        id: Some(id.clone()),
        title: Some(title.clone()),
        field: "version".to_owned(),
      }
    })?;

    let body_trimmed = body.trim();
    if body_trimmed.is_empty() {
      return Err(MarkdownError::EmptyBody {
        id: Some(id),
        title: Some(title),
      });
    }

    Ok(Self {
      id,
      title,
      description,
      instructions: body_trimmed.to_owned(),
      version,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use rstest::rstest;

  /// Valid markdown with all fields produces correct values.
  #[test]
  fn valid_markdown_with_all_fields() {
    let input = "---\nid: my-skill\ntitle: My Skill\ndescription: \
                      Does useful things\nversion: 1.0.0\n---\nStep 1: initialize\nStep 2: execute";
    let skill = DomainSkill::from_markdown(input).expect("valid markdown");
    assert_eq!(skill.id(), "my-skill");
    assert_eq!(skill.title(), "My Skill");
    assert_eq!(skill.description(), "Does useful things");
    assert_eq!(skill.version(), "1.0.0");
    assert_eq!(skill.instructions(), "Step 1: initialize\nStep 2: execute");
  }

  /// Minimal markdown — no extra code needed per field.
  #[test]
  fn minimal_markdown_builds_skill() {
    let input = "---\nid: s1\ntitle: T\ndescription: D\nversion: 2.0.0\n---\nbody content";
    let skill = DomainSkill::from_markdown(input).unwrap();
    assert_eq!(skill.id(), "s1");
    assert_eq!(skill.title(), "T");
    assert_eq!(skill.description(), "D");
    assert_eq!(skill.version(), "2.0.0");
    assert_eq!(skill.instructions(), "body content");
  }

  #[rstest]
  #[case::yaml_invalid_with_id_present("---\nid: x\n[\n---\nbody", Some("x"))]
  #[case::yaml_invalid_without_id("---\n[invalid yaml\n---\nbody", None)]
  fn yaml_parse_error_context(
    #[case] input: &str,
    #[case] expected_id: Option<&str>,
  ) {
    let err = DomainSkill::from_markdown(input).unwrap_err();
    match err {
      MarkdownError::YamlParse { id, .. } => {
        assert_eq!(id.as_deref(), expected_id);
      }
      other => panic!("Expected YamlParse, got: {other:?}"),
    }
  }

  #[test]
  fn missing_id_field() {
    let input = "---\ntitle: T\ndescription: D\nversion: 1.0.0\n---\nbody";
    let err = DomainSkill::from_markdown(input).unwrap_err();
    match err {
      MarkdownError::MissingField { field, .. } => {
        assert_eq!(field, "id");
      }
      other => panic!("Expected MissingField, got: {other:?}"),
    }
  }

  #[test]
  fn missing_title_field_with_id() {
    let input = "---\nid: x\ndescription: D\nversion: 1.0.0\n---\nbody";
    let err = DomainSkill::from_markdown(input).unwrap_err();
    match err {
      MarkdownError::MissingField { id, ref field, .. } => {
        assert_eq!(id.as_deref(), Some("x"));
        assert_eq!(field, "title");
      }
      other => panic!("Expected MissingField, got: {other:?}"),
    }
  }

  #[test]
  fn missing_description_field() {
    let input = "---\nid: x\ntitle: T\nversion: 1.0.0\n---\nbody";
    let err = DomainSkill::from_markdown(input).unwrap_err();
    match err {
      MarkdownError::MissingField {
        id,
        title,
        ref field,
        ..
      } => {
        assert_eq!(id.as_deref(), Some("x"));
        assert_eq!(title.as_deref(), Some("T"));
        assert_eq!(field, "description");
      }
      other => panic!("Expected MissingField, got: {other:?}"),
    }
  }

  #[rstest]
  #[case::empty_body_with_id(
    "---\nid: x\ntitle: T\ndescription: D\nversion: 1.0.0\n---\n",
    Some("x")
  )]
  #[case::whitespace_only_body(
    "---\nid: y\ntitle: T\ndescription: D\nversion: 1.0.0\n---\n   \n\t\n",
    Some("y")
  )]
  fn empty_or_whitespace_body(
    #[case] input: &str,
    #[case] expected_id: Option<&str>,
  ) {
    let err = DomainSkill::from_markdown(input).unwrap_err();
    match err {
      MarkdownError::EmptyBody { id, .. } => {
        assert_eq!(id.as_deref(), expected_id);
      }
      other => panic!("Expected EmptyBody, got: {other:?}"),
    }
  }

  #[rstest]
  #[case::no_opening_delimiter("id: x\ntitle: T\ndescription: D\n\nbody")]
  #[case::no_closing_delimiter("---\nid: x\ntitle: T\ndescription: D\n\nbody")]
  #[case::opening_delimiter_not_at_line_start(
    "\n---\nid: x\ntitle: T\ndescription: D\n---\nbody"
  )]
  fn invalid_delimiters(#[case] input: &str) {
    let err = DomainSkill::from_markdown(input).unwrap_err();
    assert!(
      matches!(err, MarkdownError::InvalidDelimiters { .. }),
      "Expected InvalidDelimiters, got: {err:?}"
    );
  }

  #[test]
  fn always_load_field_ignored() {
    let input = "---\nid: s\ntitle: T\ndescription: D\nversion: 1.0.0\nalways_load: \
                      true\n---\nbody";
    let skill = DomainSkill::from_markdown(input).expect("valid markdown");
    assert_eq!(skill.id(), "s");
    assert_eq!(skill.version(), "1.0.0");
    assert_eq!(skill.instructions(), "body");
  }

  #[test]
  fn extra_fields_ignored() {
    let input = "---\nid: s\ntitle: T\ndescription: D\nversion: 2\ntags: [a, \
                      b]\n---\nbody";
    let skill = DomainSkill::from_markdown(input).expect("valid markdown");
    assert_eq!(skill.id(), "s");
    assert_eq!(skill.version(), "2");
  }

  #[rstest]
  #[case::id_is_number(
    "---\nid: 123\ntitle: T\ndescription: D\nversion: 1.0.0\n---\nbody",
    "id"
  )]
  #[case::title_is_bool(
    "---\nid: x\ntitle: true\ndescription: D\nversion: 1.0.0\n---\nbody",
    "title"
  )]
  fn non_string_field_treated_as_missing(
    #[case] input: &str,
    #[case] expected_field: &str,
  ) {
    let err = DomainSkill::from_markdown(input).unwrap_err();
    match err {
      MarkdownError::MissingField { ref field, .. } => {
        assert_eq!(field, expected_field);
      }
      other => panic!("Expected MissingField, got: {other:?}"),
    }
  }

  #[test]
  fn empty_string_input() {
    let err = DomainSkill::from_markdown("").unwrap_err();
    assert!(
      matches!(err, MarkdownError::InvalidDelimiters { .. }),
      "Expected InvalidDelimiters, got: {err:?}"
    );
  }

  #[test]
  fn quoted_yaml_values() {
    let input = "---\nid: \"my-skill\"\ntitle: 'My Skill'\n\
                      description: Does stuff\nversion: \"2.0.0\"\n---\nbody";
    let skill = DomainSkill::from_markdown(input).expect("valid markdown");
    assert_eq!(skill.id(), "my-skill");
    assert_eq!(skill.title(), "My Skill");
    assert_eq!(skill.version(), "2.0.0");
  }

  #[test]
  fn crlf_line_separators() {
    let input = "---\r\nid: s\r\ntitle: T\r\ndescription: \
                      D\r\nversion: 3.0.0\r\n---\r\nbody";
    let skill = DomainSkill::from_markdown(input).expect("valid markdown");
    assert_eq!(skill.id(), "s");
    assert_eq!(skill.version(), "3.0.0");
    assert_eq!(skill.instructions(), "body");
  }
}
