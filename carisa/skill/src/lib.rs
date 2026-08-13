//! # Carisa Skill
//!
//! Skill definition, loading, and registration crate for the Carisa
//! agent orchestration platform.
//!
//! This crate provides:
//!
//! - [`DomainSkill`] — user-defined skills, buildable via
//!   [`DomainSkillBuilder`] or parsed from markdown.
//! - `PlatformSkill` — platform-defined skills with auto-load
//!   capability (`pub(crate)`).
//! - [`Skill`] enum — homogeneous storage for platform and domain
//!   skills with inherent methods (`id()`, `title()`, `description()`,
//!   `instructions()`).
//! - [`SkillMetadata`] — lightweight catalog entry (id + description).
//! - [`SkillManager`] — synchronous in-memory skill registry.
//! - [`MarkdownError`] and [`LoadError`] — error types.
//!
//! # Example
//!
//! ```rust
//! use carisa_skill::DomainSkillBuilder;
//!
//! let skill = DomainSkillBuilder::default()
//!     .id("my-skill".to_string())
//!     .title("My Skill".to_string())
//!     .description("Does something useful".to_string())
//!     .instructions("Step 1: ...".to_string())
//!     .build()
//!     .expect("all required fields set");
//!
//! assert_eq!(skill.id(), "my-skill");
//! ```

pub mod error;
pub mod manager;
pub mod markdown;
pub mod types;

pub use error::{LoadError, MarkdownError};
pub use manager::SkillManager;
pub use types::{DomainSkill, DomainSkillBuilder, Skill, SkillMetadata};
