//! Core library for the Carisa platform.
//!
//! This crate contains the shared domain types and the skill definitions used
//! by Carisa agents and orchestration layers.

pub mod agent;
pub mod identity;
pub mod memory;
pub mod skill;
pub mod telemetry;

pub use agent::{
  Agent, AgentBuilder, AgentModel, AgentPlatform, AgentPlatformBuilder,
  PlatformError, ProviderId, Secret, Transport,
};
pub use identity::{AgentId, SessionId};
pub use memory::session;
pub use skill::{
  DomainSkill, DomainSkillBuilder, LoadSkillError, MarkdownSkillError,
  PlatformSkill, PlatformSkillBuilder, SkillMetadata,
};
pub use telemetry::{
  FieldValue, MemorySink, RetryPolicy, SinkError, Telemetry, TelemetryBuilder,
  TelemetryError, TraceLevel, TraceRecord, TraceSink,
};
