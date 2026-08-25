//! Runtime value produced by a completed agent build.

use crate::{DomainSkill, PlatformSkill};

/// A concrete runtime instance built from domain and platform skill inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AgentRuntime {
  /// Domain-defined skills added to the agent context.
  pub domain_skills: Vec<DomainSkill>,
  /// Platform-defined skills available to the agent runtime.
  pub platform_skills: Vec<PlatformSkill>,
  /// Primary instructions injected into the agent prompt.
  pub instructions: String,
}

impl AgentRuntime {
  /// Creates a new runtime from its fully resolved values.
  pub const fn new(
    domain_skills: Vec<DomainSkill>,
    platform_skills: Vec<PlatformSkill>,
    instructions: String,
  ) -> Self {
    Self {
      domain_skills,
      platform_skills,
      instructions,
    }
  }
}
