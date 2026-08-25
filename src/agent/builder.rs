//! Type-state builder for constructing an agent runtime.

use crate::{DomainSkill, PlatformSkill};

use super::{
  disable::{DisabledGroup, DisabledGroups},
  filter::should_exclude_platform_skill,
  runtime::AgentRuntime,
};

/// Initial builder state.
#[derive(Debug, Clone, Default)]
pub struct AgentBuilder {
  domain_skills: Vec<DomainSkill>,
  instructions: String,
  platform_skills: Vec<PlatformSkill>,
}

impl AgentBuilder {
  /// Starts the type-state builder.
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }

  /// Replaces the current domain list and keeps the builder in the initial
  /// state.
  ///
  /// If called more than once, the list from the last call prevails.
  #[must_use]
  pub fn domain_skill(mut self, skills: Vec<DomainSkill>) -> Self {
    self.domain_skills = skills;
    self
  }

  /// Replaces the current instructions and keeps the builder in the initial
  /// state.
  ///
  /// If called more than once, the instructions from the last call prevail.
  #[must_use]
  pub fn instructions(mut self, instructions: String) -> Self {
    self.instructions = instructions;
    self
  }

  /// Stores the current platform list and transitions to the platform-aware
  /// builder state.
  ///
  /// If platform skills are configured again in the next builder state, the
  /// list from the last call prevails.
  #[must_use]
  pub fn platform_skill(
    self,
    skills: Vec<PlatformSkill>,
  ) -> AgentBuilderWithPlatform {
    AgentBuilderWithPlatform {
      domain_skills: self.domain_skills,
      instructions: self.instructions,
      platform_skills: skills,
      disabled: DisabledGroups::new(),
    }
  }

  /// Builds the final runtime without platform skills.
  ///
  /// The runtime uses the latest value configured for each field.
  pub fn build(self) -> AgentRuntime {
    AgentRuntime::new(
      self.domain_skills,
      self.platform_skills,
      self.instructions,
    )
  }
}

/// Builder state after a platform skill list has been configured.
#[derive(Debug, Clone)]
pub struct AgentBuilderWithPlatform {
  domain_skills: Vec<DomainSkill>,
  instructions: String,
  platform_skills: Vec<PlatformSkill>,
  disabled: DisabledGroups,
}

impl AgentBuilderWithPlatform {
  /// Replaces the current domain skills while keeping the platform-aware state.
  ///
  /// If called more than once, the list from the last call prevails.
  #[must_use]
  pub fn domain_skill(mut self, skills: Vec<DomainSkill>) -> Self {
    self.domain_skills = skills;
    self
  }

  /// Replaces the instructions while keeping the platform-aware state.
  ///
  /// If called more than once, the instructions from the last call prevail.
  #[must_use]
  pub fn instructions(mut self, instructions: String) -> Self {
    self.instructions = instructions;
    self
  }

  /// Replaces the platform-skills list while keeping the builder in the same
  /// state.
  ///
  /// If called more than once, the list from the last call prevails.
  #[must_use]
  pub fn platform_skill(mut self, skills: Vec<PlatformSkill>) -> Self {
    self.platform_skills = skills;
    self
  }

  /// Starts the disable-flow and transitions to the grouped disable state.
  #[must_use]
  pub fn disable(self) -> DisableBuilder {
    DisableBuilder {
      domain_skills: self.domain_skills,
      instructions: self.instructions,
      platform_skills: self.platform_skills,
      disabled: DisabledGroups::new(),
    }
  }

  /// Builds the runtime without entering or leaving the disable flow.
  ///
  /// The runtime uses the latest value configured for each field.
  pub fn build(self) -> AgentRuntime {
    build_runtime(
      self.domain_skills,
      self.platform_skills,
      self.instructions,
      &self.disabled,
    )
  }
}

/// State inside the disable flow before the final `done` call.
#[derive(Debug, Clone)]
pub struct DisableBuilder {
  domain_skills: Vec<DomainSkill>,
  instructions: String,
  platform_skills: Vec<PlatformSkill>,
  disabled: DisabledGroups,
}

impl DisableBuilder {
  /// Marks the first disabled group.
  #[must_use]
  pub fn grupo1(mut self) -> Self {
    self.disabled.insert(DisabledGroup::Grupo1);
    self
  }

  /// Marks the second disabled group.
  #[must_use]
  pub fn grupo2(mut self) -> Self {
    self.disabled.insert(DisabledGroup::Grupo2);
    self
  }

  /// Disables all groups and transitions to the `all` state.
  #[must_use]
  pub fn all(mut self) -> AllGroupsBuilder {
    self.disabled.disable_all();
    AllGroupsBuilder {
      domain_skills: self.domain_skills,
      instructions: self.instructions,
      platform_skills: self.platform_skills,
      disabled: self.disabled,
    }
  }

  /// Finishes the disable flow and returns the final builder state.
  #[must_use]
  pub fn done(self) -> AgentBuilderFinal {
    AgentBuilderFinal {
      domain_skills: self.domain_skills,
      instructions: self.instructions,
      platform_skills: self.platform_skills,
      disabled: self.disabled,
    }
  }
}

/// State reached after calling `all()` in the disable flow.
#[derive(Debug, Clone)]
pub struct AllGroupsBuilder {
  domain_skills: Vec<DomainSkill>,
  instructions: String,
  platform_skills: Vec<PlatformSkill>,
  disabled: DisabledGroups,
}

impl AllGroupsBuilder {
  /// Closes the disable flow after `all()`.
  #[must_use]
  pub fn done(self) -> AgentBuilderFinal {
    AgentBuilderFinal {
      domain_skills: self.domain_skills,
      instructions: self.instructions,
      platform_skills: self.platform_skills,
      disabled: self.disabled,
    }
  }
}

/// Builder state after the disable flow has completed.
#[derive(Debug, Clone)]
pub struct AgentBuilderFinal {
  domain_skills: Vec<DomainSkill>,
  instructions: String,
  platform_skills: Vec<PlatformSkill>,
  disabled: DisabledGroups,
}

impl AgentBuilderFinal {
  /// Replaces the current domain skills in the final state.
  ///
  /// If called more than once, the list from the last call prevails.
  #[must_use]
  pub fn domain_skill(mut self, skills: Vec<DomainSkill>) -> Self {
    self.domain_skills = skills;
    self
  }

  /// Replaces the instructions in the final state.
  ///
  /// If called more than once, the instructions from the last call prevail.
  #[must_use]
  pub fn instructions(mut self, instructions: String) -> Self {
    self.instructions = instructions;
    self
  }

  /// Builds the runtime after the disable flow has closed.
  ///
  /// The runtime uses the latest value configured for each field.
  pub fn build(self) -> AgentRuntime {
    build_runtime(
      self.domain_skills,
      self.platform_skills,
      self.instructions,
      &self.disabled,
    )
  }
}

fn build_runtime(
  domain_skills: Vec<DomainSkill>,
  platform_skills: Vec<PlatformSkill>,
  instructions: String,
  disabled: &DisabledGroups,
) -> AgentRuntime {
  let mut filtered = Vec::with_capacity(platform_skills.len());

  for skill in platform_skills {
    if !should_exclude_platform_skill(&skill, disabled) {
      filtered.push(skill);
    }
  }

  AgentRuntime::new(domain_skills, filtered, instructions)
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
    let agent = AgentBuilder::new()
      .domain_skill(vec![domain.clone()])
      .instructions("Instrucciones".to_owned())
      .build();

    assert_eq!(agent.domain_skills, vec![domain]);
    assert!(agent.platform_skills.is_empty());
    assert_eq!(agent.instructions, "Instrucciones");
  }

  #[test]
  fn build_platform_runtime_without_disable() {
    let platform_1 = make_platform("p1");
    let platform_2 = make_platform("p2");

    let agent = AgentBuilder::new()
      .platform_skill(vec![platform_1, platform_2])
      .build();

    assert_eq!(agent.platform_skills.len(), 2);
  }

  #[test]
  fn build_runtime_with_single_group_disabled() {
    let platform_1 = make_platform("p1");
    let platform_2 = make_platform("p2");

    let agent = AgentBuilder::new()
      .platform_skill(vec![platform_1, platform_2.clone()])
      .disable()
      .grupo1()
      .done()
      .build();

    assert_eq!(agent.platform_skills.len(), 1);
    assert!(agent.platform_skills.contains(&platform_2));
  }

  #[test]
  fn build_runtime_with_multiple_groups_disabled() {
    let platform_1 = make_platform("p1");
    let platform_2 = make_platform("p2");

    let agent = AgentBuilder::new()
      .platform_skill(vec![platform_1, platform_2])
      .disable()
      .grupo1()
      .grupo2()
      .done()
      .build();

    assert!(agent.platform_skills.is_empty());
  }

  #[test]
  fn build_runtime_with_all_disabled() {
    let platform_1 = make_platform("p1");
    let platform_2 = make_platform("p2");

    let agent = AgentBuilder::new()
      .platform_skill(vec![platform_1, platform_2])
      .disable()
      .all()
      .done()
      .build();

    assert!(agent.platform_skills.is_empty());
  }

  #[test]
  fn domain_skill_after_done_is_allowed() {
    let platform = make_platform("p1");
    let domain = make_domain("d1");

    let agent = AgentBuilder::new()
      .platform_skill(vec![platform])
      .disable()
      .grupo1()
      .done()
      .domain_skill(vec![domain.clone()])
      .instructions("Otras instrucciones".to_owned())
      .build();

    assert_eq!(agent.domain_skills, vec![domain]);
    assert_eq!(agent.instructions, "Otras instrucciones");
  }

  #[test]
  fn domain_skill_replacement() {
    let first = make_domain("d1");
    let second = make_domain("d2");

    let agent = AgentBuilder::new()
      .domain_skill(vec![first])
      .domain_skill(vec![second.clone()])
      .build();

    assert_eq!(agent.domain_skills, vec![second]);
  }

  #[test]
  fn platform_skill_replacement() {
    let first = make_platform("p1");
    let second = make_platform("p2");

    let agent = AgentBuilder::new()
      .platform_skill(vec![first])
      .platform_skill(vec![second.clone()])
      .build();

    assert_eq!(agent.platform_skills, vec![second]);
  }

  #[test]
  fn group_order_is_irrelevant() {
    let platform_1 = make_platform("p1");
    let platform_2 = make_platform("p2");

    let agent = AgentBuilder::new()
      .platform_skill(vec![platform_1, platform_2])
      .disable()
      .grupo2()
      .grupo1()
      .done()
      .build();

    assert!(agent.platform_skills.is_empty());
  }

  #[test]
  fn repeated_group_call_is_idempotent() {
    let platform_1 = make_platform("p1");
    let platform_2 = make_platform("p2");

    let agent = AgentBuilder::new()
      .platform_skill(vec![platform_1, platform_2.clone()])
      .disable()
      .grupo1()
      .grupo1()
      .done()
      .build();

    assert_eq!(agent.platform_skills.len(), 1);
    assert!(agent.platform_skills.contains(&platform_2));
  }
}
