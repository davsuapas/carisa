//! Core library for the Carisa platform.
//!
//! This crate contains the shared domain types and the skill definitions used
//! by Carisa agents and orchestration layers.

pub mod skill;

pub use skill::{
  DomainSkill, DomainSkillBuilder, LoadError, MarkdownError, PlatformSkill,
  PlatformSkillBuilder, Skill, SkillMetadata,
};
