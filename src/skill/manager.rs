//! Registry for the available skills in a Carisa runtime.
//!
//! The manager keeps both platform and domain skills in a single in-memory
//! index so they can be loaded, catalogued, and injected consistently.
//! [`SkillPrompt`] holds the final rendered prompt payload used by the runtime.

use std::collections::HashMap;
use std::sync::Arc;

use crate::{DomainSkill, LoadError, PlatformSkill};

/// A resolved prompt payload ready to be injected into an agent context.
///
/// The payload is stored as an owned `Arc<str>` to avoid repeated cloning of
/// the instruction text while preserving cheap read-only access.
#[derive(Debug, Clone)]
pub struct SkillPrompt {
  /// Fully rendered XML fragment containing the skill metadata and body.
  full: Arc<str>,
  /// Marks skills that must be loaded automatically for every session.
  always_load: bool,
}

impl SkillPrompt {
  /// Consumes the prompt and returns an owned string copy of its payload.
  pub fn into_owned(self) -> String {
    self.full.to_string()
  }

  /// Returns `true` when this skill should be auto-loaded into the session.
  pub const fn always_load(&self) -> bool {
    self.always_load
  }
}

/// Registry of all skills available to an agent session.
///
/// The manager is constructed once, keeps a compact in-memory index keyed by
/// skill id, and exposes a read-only API for bootstrap and catalog prompts.
#[derive(Debug)]
struct SkillManager {
  /// All loaded skills indexed by their unique identifier.
  skills: HashMap<String, SkillPrompt>,
  /// Skills that are always loaded without user request.
  bootstrap: Vec<SkillPrompt>,
  /// Optional catalog prompt listing additional, non-bootstrapped skills.
  catalog: Option<SkillPrompt>,
}

#[expect(dead_code)]
impl SkillManager {
  /// Builds a manager from platform and domain skill definitions.
  fn new(platform: Vec<PlatformSkill>, domain: Vec<DomainSkill>) -> Self {
    let mut skills =
      HashMap::with_capacity(platform.len().saturating_add(domain.len()));
    let mut bootstrap = Vec::with_capacity(platform.len());
    let mut catalog_lines =
      Vec::with_capacity(platform.len().saturating_add(domain.len()));

    for p in platform {
      let full: Arc<str> = Arc::from(format!(
        "<skill id=\"{}\" title=\"{}\" version=\"{}\">\n{}\n</skill>",
        p.id(),
        p.title(),
        p.version(),
        p.instructions()
      ));
      let always = p.always_load();

      if always {
        bootstrap.push(SkillPrompt {
          full: Arc::clone(&full),
          always_load: true,
        });
      } else {
        catalog_lines.push(format!("- id: {} — {}", p.id(), p.description()));
      }

      skills.entry(p.id().to_owned()).or_insert(SkillPrompt {
        full,
        always_load: always,
      });
    }

    for d in domain {
      let full: Arc<str> = Arc::from(format!(
        "<skill id=\"{}\" title=\"{}\" version=\"{}\">\n{}\n</skill>",
        d.id(),
        d.title(),
        d.version(),
        d.instructions()
      ));
      catalog_lines.push(format!("- id: {} — {}", d.id(), d.description()));

      skills.entry(d.id().to_owned()).or_insert(SkillPrompt {
        full,
        always_load: false,
      });
    }

    let catalog = if catalog_lines.is_empty() {
      None
    } else {
      let text = format!(
        "Tienes disponibles los siguientes skills adicionales. \
                 Usa `load_skill(id)` si la tarea lo requiere.\n\n{}",
        catalog_lines.join("\n")
      );
      Some(SkillPrompt {
        full: Arc::from(text),
        always_load: false,
      })
    };

    Self {
      skills,
      bootstrap,
      catalog,
    }
  }

  /// Returns the prompt slice that is loaded into every agent session.
  fn bootstrap_prompts(&self) -> &[SkillPrompt] {
    &self.bootstrap
  }

  /// Returns the optional catalog prompt, if the manager has additional skills.
  fn catalog_prompt(&self) -> Option<SkillPrompt> {
    self.catalog.clone()
  }

  /// Loads a skill definition by its unique identifier.
  ///
  /// # Errors
  ///
  /// Returns [`LoadError`] when the requested skill id does not exist in the
  /// registry.
  fn load(&self, id: &str) -> Result<SkillPrompt, LoadError> {
    self
      .skills
      .get(id)
      .cloned()
      .ok_or_else(|| LoadError { id: id.to_owned() })
  }
}

impl Default for SkillManager {
  fn default() -> Self {
    Self::new(vec![], vec![])
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::skill::types::{DomainSkillBuilder, PlatformSkillBuilder};

  #[test]
  fn excludes_always_load_from_catalog() {
    let ps_always = PlatformSkillBuilder::default()
      .id("core".to_owned())
      .title("Core".to_owned())
      .description("Core description".to_owned())
      .instructions("Do core things".to_owned())
      .version("1.0.0".to_owned())
      .always_load(true)
      .build()
      .expect("always-load platform skill");

    let ps_normal = PlatformSkillBuilder::default()
      .id("helper".to_owned())
      .title("Helper".to_owned())
      .description("Helper description".to_owned())
      .instructions("Help the user".to_owned())
      .version("1.1.0".to_owned())
      .always_load(false)
      .build()
      .expect("normal platform skill");

    let manager = SkillManager::new(vec![ps_always, ps_normal], vec![]);
    let catalog = manager
      .catalog_prompt()
      .expect("catalog prompt")
      .into_owned();

    assert!(catalog.contains("helper"));
    assert!(catalog.contains("Helper description"));
    assert!(!catalog.contains("core"));
  }

  #[test]
  fn multiple_domain_skills_in_catalog() {
    let a = DomainSkillBuilder::default()
      .id("a".to_owned())
      .title("A".to_owned())
      .description("Skill A".to_owned())
      .instructions("Do A".to_owned())
      .version("1.0.0".to_owned())
      .build()
      .expect("domain skill a");

    let b = DomainSkillBuilder::default()
      .id("b".to_owned())
      .title("B".to_owned())
      .description("Skill B".to_owned())
      .instructions("Do B".to_owned())
      .version("1.0.1".to_owned())
      .build()
      .expect("domain skill b");

    let c = DomainSkillBuilder::default()
      .id("c".to_owned())
      .title("C".to_owned())
      .description("Skill C".to_owned())
      .instructions("Do C".to_owned())
      .version("2.0.0".to_owned())
      .build()
      .expect("domain skill c");

    let manager = SkillManager::new(vec![], vec![a, b, c]);
    let catalog = manager
      .catalog_prompt()
      .expect("catalog prompt")
      .into_owned();

    assert!(catalog.contains("- id: a"));
    assert!(catalog.contains("- id: b"));
    assert!(catalog.contains("- id: c"));
  }

  #[test]
  fn empty_catalog_when_no_skills() {
    let manager = SkillManager::new(vec![], vec![]);
    assert!(manager.catalog_prompt().is_none());
  }

  #[test]
  fn load_returns_instructions_without_clone() {
    let skill = DomainSkillBuilder::default()
      .id("guide".to_owned())
      .title("Guide".to_owned())
      .description("A guide skill".to_owned())
      .instructions("Do X then Y".to_owned())
      .version("9.9.9".to_owned())
      .build()
      .expect("domain skill");

    let manager = SkillManager::new(vec![], vec![skill]);

    let instructions = manager.load("guide").unwrap().into_owned();
    assert!(instructions.contains("Do X then Y"));
  }

  #[test]
  fn load_returns_error_for_missing_id() {
    let skill = DomainSkillBuilder::default()
      .id("exists".to_owned())
      .title("Exists".to_owned())
      .description("An existing skill".to_owned())
      .instructions("Do something".to_owned())
      .version("7.7.7".to_owned())
      .build()
      .expect("domain skill");

    let manager = SkillManager::new(vec![], vec![skill]);

    let result = manager.load("no-existe");
    assert!(result.is_err());
    assert_eq!(
      result.unwrap_err(),
      LoadError {
        id: "no-existe".to_owned()
      }
    );
  }

  #[test]
  fn platform_duplicate_id_precedence() {
    let ps1 = PlatformSkillBuilder::default()
      .id("dup".to_owned())
      .title("Dup".to_owned())
      .description("First".to_owned())
      .instructions("primero".to_owned())
      .version("1.0.0".to_owned())
      .always_load(false)
      .build()
      .expect("first platform skill");

    let ps2 = PlatformSkillBuilder::default()
      .id("dup".to_owned())
      .title("Dup".to_owned())
      .description("Second".to_owned())
      .instructions("segundo".to_owned())
      .version("1.1.0".to_owned())
      .always_load(false)
      .build()
      .expect("second platform skill");

    let manager = SkillManager::new(vec![ps1, ps2], vec![]);

    let loaded = manager.load("dup").unwrap().into_owned();
    assert!(loaded.contains("primero"));
    assert!(!loaded.contains("segundo"));
    assert!(manager.catalog_prompt().is_some());
  }

  #[test]
  fn domain_duplicate_id_precedence() {
    let ds1 = DomainSkillBuilder::default()
      .id("dup".to_owned())
      .title("Dup".to_owned())
      .description("First".to_owned())
      .instructions("first".to_owned())
      .version("1.0.0".to_owned())
      .build()
      .expect("first domain skill");

    let ds2 = DomainSkillBuilder::default()
      .id("dup".to_owned())
      .title("Dup".to_owned())
      .description("Second".to_owned())
      .instructions("second".to_owned())
      .version("2.0.0".to_owned())
      .build()
      .expect("second domain skill");

    let manager = SkillManager::new(vec![], vec![ds1, ds2]);

    let loaded = manager.load("dup").unwrap().into_owned();
    assert!(loaded.contains("first"));
    assert!(!loaded.contains("second"));
    assert!(manager.catalog_prompt().is_some());
  }

  #[test]
  fn platform_wins_over_domain_on_same_id() {
    let ps = PlatformSkillBuilder::default()
      .id("shared".to_owned())
      .title("Shared".to_owned())
      .description("Platform".to_owned())
      .instructions("platform".to_owned())
      .version("1.0.0".to_owned())
      .always_load(false)
      .build()
      .expect("platform skill");

    let ds = DomainSkillBuilder::default()
      .id("shared".to_owned())
      .title("Shared".to_owned())
      .description("Domain".to_owned())
      .instructions("domain".to_owned())
      .version("2.0.0".to_owned())
      .build()
      .expect("domain skill");

    let manager = SkillManager::new(vec![ps], vec![ds]);

    let loaded = manager.load("shared").unwrap().into_owned();
    assert!(loaded.contains("platform"));
    assert!(!loaded.contains("domain"));
    assert!(manager.catalog_prompt().is_some());
  }

  #[test]
  fn empty_vectors_produce_empty_manager() {
    let manager = SkillManager::new(vec![], vec![]);

    assert!(manager.catalog_prompt().is_none());
    assert!(manager.load("anything").is_err());
  }

  #[test]
  fn mixed_platform_and_domain_skills() {
    let ps_always = PlatformSkillBuilder::default()
      .id("core".to_owned())
      .title("Core".to_owned())
      .description("Core description".to_owned())
      .instructions("Do core things".to_owned())
      .version("1.0.0".to_owned())
      .always_load(true)
      .build()
      .expect("always-load platform skill");

    let ps_normal = PlatformSkillBuilder::default()
      .id("helper".to_owned())
      .title("Helper".to_owned())
      .description("Helper description".to_owned())
      .instructions("Help the user".to_owned())
      .version("1.1.0".to_owned())
      .always_load(false)
      .build()
      .expect("normal platform skill");

    let ds = DomainSkillBuilder::default()
      .id("custom".to_owned())
      .title("Custom".to_owned())
      .description("Custom description".to_owned())
      .instructions("Do custom stuff".to_owned())
      .version("2.0.0".to_owned())
      .build()
      .expect("domain skill");

    let manager = SkillManager::new(vec![ps_always, ps_normal], vec![ds]);
    let catalog = manager
      .catalog_prompt()
      .expect("catalog prompt")
      .into_owned();

    assert!(catalog.contains("helper"));
    assert!(catalog.contains("custom"));
    assert!(!catalog.contains("core"));
  }

  #[test]
  fn arc_skill_manager() {
    use std::sync::Arc;

    let skill = DomainSkillBuilder::default()
      .id("x".to_owned())
      .title("X".to_owned())
      .description("Skill X".to_owned())
      .instructions("Do X".to_owned())
      .version("3.0.0".to_owned())
      .build()
      .expect("domain skill");

    let manager = SkillManager::new(vec![], vec![skill]);
    let shared = Arc::new(manager);

    let instructions = shared.load("x").unwrap().into_owned();
    assert!(instructions.contains("Do X"));

    let catalog = shared
      .catalog_prompt()
      .expect("catalog prompt")
      .into_owned();
    assert!(catalog.contains('x'));
  }

  #[test]
  fn debug_and_default_traits() {
    let manager = SkillManager::default();
    assert!(manager.catalog_prompt().is_none());
    drop(format!("{manager:?}"));
  }
}
