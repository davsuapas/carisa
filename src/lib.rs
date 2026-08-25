//! Core library for the Carisa platform.
//!
//! This crate contains the shared domain types and the skill definitions used
//! by Carisa agents and orchestration layers.

pub mod agent;
pub mod skill;

pub use agent::{
  AgentBuilder, AgentBuilderFinal, AgentBuilderWithPlatform, AgentRuntime,
  AllGroupsBuilder, DisableBuilder,
};
pub use skill::{
  DomainSkill, DomainSkillBuilder, LoadError, MarkdownError, PlatformSkill,
  PlatformSkillBuilder, Skill, SkillMetadata,
};
