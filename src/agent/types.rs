//! Agent definitions and builders for runtime use.

use std::collections::HashMap;

use derive_builder::Builder;
use serde::{Deserialize, Serialize};

use crate::{DomainSkill, PlatformSkill, agent::config::Generation};

/// Model configuration selected for an agent.
#[derive(Builder, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[builder(pattern = "owned")]
#[non_exhaustive]
pub struct AgentModel {
  /// Model identifier.
  id: String,
  /// Generation options specific to this model selection.
  #[serde(default)]
  #[builder(default, setter(into, strip_option))]
  generation: Option<Generation>,
  /// Additional parameters specific to this model selection.
  #[serde(default)]
  #[builder(default, setter(into, strip_option))]
  additional_params: Option<HashMap<String, serde_json::Value>>,
}

impl AgentModel {
  /// Creates a model configuration with no overrides.
  pub fn new(id: impl Into<String>) -> Self {
    Self {
      id: id.into(),
      generation: None,
      additional_params: None,
    }
  }

  /// Returns the model identifier.
  pub fn id(&self) -> &str {
    &self.id
  }

  /// Returns the optional generation configuration.
  pub const fn generation(&self) -> Option<&Generation> {
    self.generation.as_ref()
  }

  /// Returns the additional parameters.
  pub const fn additional_params(
    &self,
  ) -> Option<&HashMap<String, serde_json::Value>> {
    self.additional_params.as_ref()
  }
}

impl From<String> for AgentModel {
  fn from(id: String) -> Self {
    Self::new(id)
  }
}

impl From<&str> for AgentModel {
  fn from(id: &str) -> Self {
    Self::new(id)
  }
}

/// Initial builder state.
#[derive(Builder, Debug, Clone, Default, Serialize, Deserialize)]
pub struct Agent {
  /// Instructions for the agent.
  instructions: String,
  /// Domain skills for the agent.
  #[builder(default)]
  domain_skills: Vec<DomainSkill>,
  /// Platform skills for the agent.
  #[builder(default)]
  platform_skills: Vec<PlatformSkill>,
  /// Optional explicit model binding for this agent.
  #[builder(default, setter(into, strip_option))]
  model: Option<AgentModel>,
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
  pub fn platform_skills(&self) -> &[PlatformSkill] {
    &self.platform_skills
  }

  /// Returns the optional model configuration for the agent.
  pub const fn model(&self) -> Option<&AgentModel> {
    self.model.as_ref()
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
    assert_eq!(agent.platform_skills().len(), 0);
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

  #[test]
  fn model_accepts_identifier_and_optional_overrides() {
    let mut additional_params = HashMap::new();
    additional_params.insert("reasoning_effort".to_owned(), "high".into());
    let model = AgentModelBuilder::default()
      .id("model-id".to_owned())
      .generation(Generation::default())
      .additional_params(additional_params.clone())
      .build()
      .expect("model identifier provided");
    let agent = AgentBuilder::default()
      .instructions("Instructions".to_owned())
      .model(model)
      .build()
      .expect("instructions provided");

    assert_eq!(agent.model.as_ref().unwrap().id, "model-id");
    assert_eq!(
      agent.model.unwrap().additional_params,
      Some(additional_params)
    );
  }

  #[test]
  fn model_overrides_are_optional_when_building_and_deserializing() {
    let built_model = AgentModelBuilder::default()
      .id("model-id".to_owned())
      .build()
      .expect("model identifier is sufficient");

    assert!(built_model.generation.is_none());
    assert!(built_model.additional_params.is_none());

    let model: AgentModel = serde_json::from_value(serde_json::json!({
      "id": "model-id"
    }))
    .expect("model identifier is sufficient");

    assert_eq!(model.id, "model-id");
    assert!(model.generation.is_none());
    assert!(model.additional_params.is_none());
  }
}
