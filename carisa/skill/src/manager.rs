//! Skill manager — central registry for platform and domain skills.
//!
//! Provides [`SkillManager`], a synchronous in-memory store backed by a
//! [`HashMap`] that maps skill ids to [`Skill`] values. The manager
//! receives all skills at construction time and is immutable afterwards.
//!
//! The manager is `Send + Sync` by design and can be wrapped in
//! `Arc<SkillManager>` for shared read-only access in concurrent
//! contexts.

use std::collections::HashMap;

use crate::error::LoadError;
use crate::types::{DomainSkill, PlatformSkill, Skill, SkillMetadata};

/// Central registry for skill definitions.
///
/// Stores skills in a `HashMap<String, Skill>` keyed by skill id.
/// Skills are provided at construction time via [`new`](Self::new).
/// When a duplicate id appears across or within the input vectors,
/// the first occurrence takes precedence; later entries with the
/// same id are silently ignored.
#[derive(Debug)]
pub struct SkillManager {
    skills: HashMap<String, Skill>,
}

impl SkillManager {
    /// Creates a new [`SkillManager`] with the given platform and domain
    /// skills.
    ///
    /// Platform skills are inserted first, so they take precedence over
    /// domain skills with the same id. Within each vector, earlier entries
    /// take precedence over later ones with the same id.
    pub fn new(platform_skills: Vec<PlatformSkill>, domain_skills: Vec<DomainSkill>) -> Self {
        let mut skills =             HashMap::with_capacity(
                platform_skills.len().saturating_add(domain_skills.len()),
            );

        for skill in platform_skills {
            let id = skill.id().to_owned();
            skills.entry(id).or_insert_with(|| Skill::Platform(skill));
        }

        for skill in domain_skills {
            let id = skill.id().to_owned();
            skills.entry(id).or_insert_with(|| Skill::Domain(skill));
        }

        Self { skills }
    }

    /// Returns a catalog of all user-visible skills.
    ///
    /// Platform skills with `always_load = true` are excluded from the
    /// catalog because they are loaded automatically and should not
    /// appear in the user-facing list.
    pub fn catalog(&self) -> Vec<SkillMetadata> {
        let mut result = Vec::new();
        for skill in self.skills.values() {
            match skill {
                Skill::Platform(p) => {
                    if !p.always_load() {
                        result.push(SkillMetadata {
                            id: p.id().to_owned(),
                            description: p.description().to_owned(),
                        });
                    }
                }
                Skill::Domain(d) => {
                    result.push(SkillMetadata {
                        id: d.id().to_owned(),
                        description: d.description().to_owned(),
                    });
                }
            }
        }
        result
    }

    /// Loads a skill by its id.
    ///
    /// Returns a reference to the stored [`Skill`] on success, or
    /// [`LoadError`] if the id is not registered.
    ///
    /// # Errors
    ///
    /// Returns `Err(LoadError)` when no skill with the requested `id`
    /// has been registered.
    pub fn load(&self, id: &str) -> Result<&Skill, LoadError> {
        self.skills
            .get(id)
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
    use crate::types::{DomainSkillBuilder, PlatformSkillBuilder};

    #[test]
    fn excludes_always_load_from_catalog() {
        let ps_always = PlatformSkillBuilder::default()
            .id("core".to_owned())
            .title("Core".to_owned())
            .description("Core description".to_owned())
            .instructions("Do core things".to_owned())
            .always_load(true)
            .build()
            .expect("always-load platform skill");

        let ps_normal = PlatformSkillBuilder::default()
            .id("helper".to_owned())
            .title("Helper".to_owned())
            .description("Helper description".to_owned())
            .instructions("Help the user".to_owned())
            .always_load(false)
            .build()
            .expect("normal platform skill");

        let manager = SkillManager::new(vec![ps_always, ps_normal], vec![]);
        let catalog = manager.catalog();

        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog.first().unwrap().id, "helper");
        assert_eq!(catalog.first().unwrap().description, "Helper description");
    }

    #[test]
    fn multiple_domain_skills_in_catalog() {
        let a = DomainSkillBuilder::default()
            .id("a".to_owned())
            .title("A".to_owned())
            .description("Skill A".to_owned())
            .instructions("Do A".to_owned())
            .build()
            .expect("domain skill a");

        let b = DomainSkillBuilder::default()
            .id("b".to_owned())
            .title("B".to_owned())
            .description("Skill B".to_owned())
            .instructions("Do B".to_owned())
            .build()
            .expect("domain skill b");

        let c = DomainSkillBuilder::default()
            .id("c".to_owned())
            .title("C".to_owned())
            .description("Skill C".to_owned())
            .instructions("Do C".to_owned())
            .build()
            .expect("domain skill c");

        let manager = SkillManager::new(vec![], vec![a, b, c]);
        let catalog = manager.catalog();

        assert_eq!(catalog.len(), 3);
        assert!(catalog.iter().any(|m| m.id == "a"));
        assert!(catalog.iter().any(|m| m.id == "b"));
        assert!(catalog.iter().any(|m| m.id == "c"));
    }

    #[test]
    fn empty_catalog_when_no_skills() {
        let manager = SkillManager::new(vec![], vec![]);
        assert!(manager.catalog().is_empty());
    }

    #[test]
    fn load_returns_instructions_without_clone() {
        let skill = DomainSkillBuilder::default()
            .id("guide".to_owned())
            .title("Guide".to_owned())
            .description("A guide skill".to_owned())
            .instructions("Do X then Y".to_owned())
            .build()
            .expect("domain skill");

        let manager = SkillManager::new(vec![], vec![skill]);

        let instructions = manager.load("guide").unwrap().instructions();
        assert_eq!(instructions, "Do X then Y");
    }

    #[test]
    fn load_returns_error_for_missing_id() {
        let skill = DomainSkillBuilder::default()
            .id("exists".to_owned())
            .title("Exists".to_owned())
            .description("An existing skill".to_owned())
            .instructions("Do something".to_owned())
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
            .always_load(false)
            .build()
            .expect("first platform skill");

        let ps2 = PlatformSkillBuilder::default()
            .id("dup".to_owned())
            .title("Dup".to_owned())
            .description("Second".to_owned())
            .instructions("segundo".to_owned())
            .always_load(false)
            .build()
            .expect("second platform skill");

        let manager = SkillManager::new(vec![ps1, ps2], vec![]);

        assert_eq!(manager.load("dup").unwrap().instructions(), "primero");
        assert_eq!(manager.catalog().len(), 1);
    }

    #[test]
    fn domain_duplicate_id_precedence() {
        let ds1 = DomainSkillBuilder::default()
            .id("dup".to_owned())
            .title("Dup".to_owned())
            .description("First".to_owned())
            .instructions("first".to_owned())
            .build()
            .expect("first domain skill");

        let ds2 = DomainSkillBuilder::default()
            .id("dup".to_owned())
            .title("Dup".to_owned())
            .description("Second".to_owned())
            .instructions("second".to_owned())
            .build()
            .expect("second domain skill");

        let manager = SkillManager::new(vec![], vec![ds1, ds2]);

        assert_eq!(manager.load("dup").unwrap().instructions(), "first");
        assert_eq!(manager.catalog().len(), 1);
    }

    #[test]
    fn platform_wins_over_domain_on_same_id() {
        let ps = PlatformSkillBuilder::default()
            .id("shared".to_owned())
            .title("Shared".to_owned())
            .description("Platform".to_owned())
            .instructions("platform".to_owned())
            .always_load(false)
            .build()
            .expect("platform skill");

        let ds = DomainSkillBuilder::default()
            .id("shared".to_owned())
            .title("Shared".to_owned())
            .description("Domain".to_owned())
            .instructions("domain".to_owned())
            .build()
            .expect("domain skill");

        let manager = SkillManager::new(vec![ps], vec![ds]);

        assert_eq!(manager.load("shared").unwrap().instructions(), "platform");
        assert_eq!(manager.catalog().len(), 1);
    }

    #[test]
    fn empty_vectors_produce_empty_manager() {
        let manager = SkillManager::new(vec![], vec![]);

        assert!(manager.catalog().is_empty());
        assert!(manager.load("anything").is_err());
    }

    #[test]
    fn mixed_platform_and_domain_skills() {
        let ps_always = PlatformSkillBuilder::default()
            .id("core".to_owned())
            .title("Core".to_owned())
            .description("Core description".to_owned())
            .instructions("Do core things".to_owned())
            .always_load(true)
            .build()
            .expect("always-load platform skill");

        let ps_normal = PlatformSkillBuilder::default()
            .id("helper".to_owned())
            .title("Helper".to_owned())
            .description("Helper description".to_owned())
            .instructions("Help the user".to_owned())
            .always_load(false)
            .build()
            .expect("normal platform skill");

        let ds = DomainSkillBuilder::default()
            .id("custom".to_owned())
            .title("Custom".to_owned())
            .description("Custom description".to_owned())
            .instructions("Do custom stuff".to_owned())
            .build()
            .expect("domain skill");

        let manager = SkillManager::new(vec![ps_always, ps_normal], vec![ds]);
        let catalog = manager.catalog();

        assert_eq!(catalog.len(), 2);
        assert!(catalog.iter().any(|m| m.id == "helper"));
        assert!(catalog.iter().any(|m| m.id == "custom"));
        assert!(!catalog.iter().any(|m| m.id == "core"));
    }

    #[test]
    fn arc_skill_manager() {
        use std::sync::Arc;

        let skill = DomainSkillBuilder::default()
            .id("x".to_owned())
            .title("X".to_owned())
            .description("Skill X".to_owned())
            .instructions("Do X".to_owned())
            .build()
            .expect("domain skill");

        let manager = SkillManager::new(vec![], vec![skill]);
        let shared = Arc::new(manager);

        let instructions = shared.load("x").unwrap().instructions().to_owned();
        assert_eq!(instructions, "Do X");

        let catalog = shared.catalog();
        assert_eq!(catalog.len(), 1);
    }

    #[test]
    fn debug_and_default_traits() {
        let manager = SkillManager::default();
        assert!(manager.catalog().is_empty());
        drop(format!("{manager:?}"));
    }
}
