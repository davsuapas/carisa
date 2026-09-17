//! Core library for the Carisa platform.
//!
//! This crate contains the shared domain types and the skill definitions used
//! by Carisa agents and orchestration layers.

pub mod agent;
pub mod memory;
pub mod skill;

pub use agent::builder::AgentBuilder;
pub use agent::runtime::AgentRuntime;
pub use memory::log;
pub use skill::{
  DomainSkill, DomainSkillBuilder, LoadSkillError, MarkdownSkillError,
  PlatformSkill, PlatformSkillBuilder, Skill, SkillMetadata,
};
