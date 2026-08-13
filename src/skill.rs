//! Skill definitions and helpers for the Carisa agent platform.
//!
//! The system supports two kinds of skills:
//!
//! - Platform skills are built-in capabilities that ship with the runtime and
//!   are intended for core behavior and bootstrap scenarios.
//! - Domain skills are user-defined or imported skills that represent custom
//!   behavior for a specific domain, typically created via the builder API or
//!   parsed from markdown.
//!
//! The registry that coordinates them keeps a single index of both kinds and
//! exposes the prompt data needed during orchestration.
//!
//! # Example (domain skill via markdown)
//!
//! ```markdown
//! ---
//! id: support-agent
//! title: Support Agent
//! description: Responds to user support requests.
//! version: 1.0.0
//! ---
//! Step 1: identify the issue.
//! Step 2: suggest a safe response.
//! ```
//!
//! ```rust
//! use carisa_core::DomainSkill;
//!
//! let skill = DomainSkill::from_markdown(
//!   "---\n\
//!   id: support-agent\n\
//!   title: Support Agent\n\
//!   description: Responds to user support requests.\n\
//!   version: 1.0.0\n\
//!   ---\n\
//!   Step 1: identify the issue.\n\
//!   Step 2: suggest a safe response.\n",
//! )
//! .expect("valid skill markdown");
//!
//! assert_eq!(skill.id(), "support-agent");
//! ```

pub mod error;
mod manager;
pub mod markdown;
pub mod types;

pub use error::{LoadError, MarkdownError};
pub use manager::SkillPrompt;
pub use types::{
  DomainSkill, DomainSkillBuilder, PlatformSkill, PlatformSkillBuilder, Skill,
  SkillMetadata,
};
