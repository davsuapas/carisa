//! Construction of an agent runtime and its initial system message.
//!
//! The runtime combines the agent's instructions with its skill manager. Its
//! preamble is a Rig system message, kept separate from conversation history.

use rig_core::completion::Message;

use crate::{agent::types::Agent, skill::manager::SkillManager};

/// Runtime state for one configured agent.
#[cfg_attr(not(test), expect(dead_code))]
#[derive(Debug)]
pub(super) struct AgentRuntime {
  preamble: Message,
  #[cfg_attr(test, expect(dead_code))]
  skill_manager: SkillManager,
}

#[cfg_attr(not(test), expect(dead_code))]
impl AgentRuntime {
  /// Builds a runtime from an agent and its skill definitions.
  #[must_use]
  pub(crate) fn from_agent(agent: Agent) -> Self {
    let (mut instructions, platform_skills, domain_skills) =
      agent.into_runtime_parts();
    let skill_manager = SkillManager::new(platform_skills, domain_skills);
    let bootstrap = skill_manager.bootstrap_prompts();
    let mut prefix = String::new();

    if let Some(catalog) = skill_manager.catalog_prompt() {
      prefix.push_str(catalog.as_str());
    }
    for prompt in bootstrap {
      if !prefix.is_empty() {
        prefix.push_str("\n\n");
      }
      prefix.push_str(prompt.as_str());
    }
    if !prefix.is_empty() {
      prefix.push_str("\n\n");
      instructions.reserve(prefix.len());
      instructions.insert_str(0, &prefix);
    }

    Self {
      preamble: Message::system(instructions),
      skill_manager,
    }
  }

  /// Borrows the initial system message prepared for Rig.
  pub(crate) const fn preamble(&self) -> &Message {
    &self.preamble
  }

  /// Moves the initial system message out for Rig or session storage.
  ///
  /// The optional skill catalog comes first, followed by skills marked
  /// `always_load`, and the agent's instructions last. This message is not
  /// conversation history; wrap it with
  /// [`LogMessage::preamble_message`](crate::memory::session::LogMessage::preamble_message)
  /// when storing it.
  #[must_use]
  pub(crate) fn into_preamble(self) -> Message {
    self.preamble
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    AgentBuilder, DomainSkill, PlatformSkill,
    skill::types::{DomainSkillBuilder, PlatformSkillBuilder},
  };

  fn platform_skill(id: &str, always_load: bool) -> PlatformSkill {
    PlatformSkillBuilder::default()
      .id(id.to_owned())
      .title(format!("{id} title"))
      .description(format!("{id} description"))
      .instructions(format!("{id} instructions"))
      .version("1.0.0".to_owned())
      .always_load(always_load)
      .build()
      .expect("valid platform skill")
  }

  fn domain_skill(id: &str) -> DomainSkill {
    DomainSkillBuilder::default()
      .id(id.to_owned())
      .title(format!("{id} title"))
      .description(format!("{id} description"))
      .instructions(format!("{id} instructions"))
      .version("1.0.0".to_owned())
      .build()
      .expect("valid domain skill")
  }

  fn system_content(message: &Message) -> &str {
    let Message::System { content } = message else {
      panic!("expected a system message");
    };
    content
  }

  #[test]
  fn preamble_orders_catalog_bootstrap_and_agent_instructions() {
    let agent = AgentBuilder::default()
      .instructions("agent instructions".to_owned())
      .platform_skills(vec![
        platform_skill("catalog-skill", false),
        platform_skill("bootstrap-one", true),
        platform_skill("bootstrap-two", true),
      ])
      .domain_skills(vec![domain_skill("domain-skill")])
      .build()
      .expect("agent instructions are provided");
    let runtime = AgentRuntime::from_agent(agent);
    let preamble = system_content(runtime.preamble());

    let catalog = preamble
      .find("- id: catalog-skill")
      .expect("catalog comes first");
    let first_bootstrap = preamble
      .find("<skill id=\"bootstrap-one\"")
      .expect("first bootstrap skill is included");
    let second_bootstrap = preamble
      .find("<skill id=\"bootstrap-two\"")
      .expect("second bootstrap skill is included");
    let instructions = preamble
      .find("agent instructions")
      .expect("agent instructions are included");

    assert!(catalog < first_bootstrap);
    assert!(first_bootstrap < second_bootstrap);
    assert!(second_bootstrap < instructions);
    assert!(preamble.contains("- id: domain-skill"));
    assert!(!preamble.contains("<skill id=\"catalog-skill\""));
    assert!(!preamble.contains("<skill id=\"domain-skill\""));
  }

  #[test]
  fn preamble_without_skills_contains_only_instructions() {
    let agent = AgentBuilder::default()
      .instructions("only instructions".to_owned())
      .build()
      .expect("agent instructions are provided");
    let runtime = AgentRuntime::from_agent(agent);

    assert_eq!(system_content(runtime.preamble()), "only instructions");
    assert_eq!(
      system_content(&runtime.into_preamble()),
      "only instructions"
    );
  }
}
