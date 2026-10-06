//! Core library for the Carisa platform.
//!
//! This crate contains the shared domain types and the skill definitions used
//! by Carisa agents and orchestration layers.

pub mod agent;
pub mod identity;
pub mod memory;
pub mod runtime;
pub mod skill;

pub use agent::{Agent, AgentBuilder, AgentModel};
pub use identity::{AgentId, SessionId};
pub use memory::session;
pub use runtime::{
  AgentPlatform, AgentPlatformBuilder, PlatformError, ProviderKind, Secret,
};
pub use skill::{
  DomainSkill, DomainSkillBuilder, LoadSkillError, MarkdownSkillError,
  PlatformSkill, PlatformSkillBuilder, SkillMetadata,
};
