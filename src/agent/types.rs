//! Agent types and builders.

use derive_builder::Builder;

use crate::{DomainSkill, PlatformSkill};

/// Initial builder state.
#[derive(Builder, Debug, Clone, Default)]
pub struct Agent {
  /// Instructions for the agent.
  instructions: String,
  /// Domain skills for the agent.
  #[builder(default)]
  domain_skills: Vec<DomainSkill>,
  /// Platform skills for the agent.
  #[builder(default)]
  platform_skills: Vec<PlatformSkill>,
}

impl Agent {
  /// Returns the instructions for the agent.
  pub fn instructions(&self) -> &str {
    &self.instructions
  }

  /// Returns the domain skills for the agent.
  pub fn domain_skills(&self) -> &[DomainSkill] {
    &self.domain_skills
  }

  /// Returns the platform skills for the agent.
  ///
  /// Drops any standard platform skills  
  pub fn platform_skills(&self) -> &[PlatformSkill] {
    &self.platform_skills
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::skill::types::{DomainSkillBuilder, PlatformSkillBuilder};

  fn make_platform(id: &str) -> PlatformSkill {
    PlatformSkillBuilder::default()
      .id(id.to_owned())
      .title(format!("{id} Title"))
      .description(format!("{id} description"))
      .instructions(format!("Instructions for {id}"))
      .version("1.0.0".to_owned())
      .always_load(false)
      .build()
      .expect("valid platform skill")
  }

  fn make_domain(id: &str) -> DomainSkill {
    DomainSkillBuilder::default()
      .id(id.to_owned())
      .title(format!("{id} title"))
      .description(format!("{id} description"))
      .instructions(format!("Instructions for {id}"))
      .version("1.0.0".to_owned())
      .build()
      .expect("valid domain skill")
  }

  #[test]
  fn build_domain_only_runtime() {
    let domain = make_domain("d1");
    let agent = AgentBuilder::default()
      .domain_skills(vec![domain.clone()])
      .instructions("Instrucciones".to_owned())
      .build()
      .expect("all fields provided");

    assert_eq!(agent.domain_skills(), vec![domain]);
    assert!(agent.platform_skills().is_empty());
    assert_eq!(agent.instructions, "Instrucciones");
  }

  #[test]
  fn build_platform_runtime_without_disable() {
    let platform_1 = make_platform("p1");
    let platform_2 = make_platform("p2");

    let agent = AgentBuilder::default()
      .instructions("Platform runtime instructions".to_owned())
      .platform_skills(vec![platform_1, platform_2])
      .build()
      .expect("all fields provided");

    assert_eq!(agent.platform_skills.len(), 2);
  }

  #[test]
  fn domain_skill_replacement() {
    let first = make_domain("d1");
    let second = make_domain("d2");

    let agent = AgentBuilder::default()
      .instructions("Replacement instructions".to_owned())
      .domain_skills(vec![first])
      .domain_skills(vec![second.clone()])
      .build()
      .expect("all fields provided");

    assert_eq!(agent.domain_skills, vec![second]);
  }

  #[test]
  fn platform_skill_replacement() {
    let first = make_platform("p1");
    let second = make_platform("p2");

    let agent = AgentBuilder::default()
      .instructions("Platform replacement instructions".to_owned())
      .platform_skills(vec![first])
      .platform_skills(vec![second.clone()])
      .build()
      .expect("all fields provided");

    assert_eq!(agent.platform_skills, vec![second]);
  }
}
