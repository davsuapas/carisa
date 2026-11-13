//! Agent definitions and runtime configuration for constructing an agent
//! platform.
//!
//! Create agents first with [`crate::AgentBuilder`], then pass the completed
//! agents to [`AgentPlatformBuilder`]. The platform builder combines
//! provider, HTTP, model, default, agent, and log-storage settings, then
//! [`AgentPlatformBuilder::build`] validates references and creates an
//! [`AgentPlatform`]. Providers and models must be configured in code, a JSON
//! file, or environment variables. Agents and log storage are supplied to
//! the builder in Rust.
//!
//! # Creating agents
//!
//! An [`crate::Agent`] groups required instructions with optional domain
//! skills, platform skills, and an explicit model selection. The agent is a
//! configuration value; it does not load skills or execute model requests.
//! Instructions are required. Skills and the model are optional. A domain
//! skill describes a reusable, domain-specific capability; a platform skill
//! represents a runtime capability and can be marked to load automatically
//! for every agent session. Build skills separately and provide them to the
//! corresponding builder fields.
//!
//! ```rust
//! use carisa_core::{AgentBuilder, AgentModel, DomainSkillBuilder};
//!
//! let research_skill = DomainSkillBuilder::default()
//!   .id("research".to_owned())
//!   .title("Research".to_owned())
//!   .description("Find and compare relevant sources.".to_owned())
//!   .instructions("Check reliable sources.".to_owned())
//!   .version("1.0.0".to_owned())
//!   .build()
//!   .expect("all required skill fields are set");
//!
//! let agent = AgentBuilder::default()
//!   .instructions("Answer clearly and cite relevant evidence.".to_owned())
//!   .domain_skills(vec![research_skill])
//!   .model(AgentModel::new("gpt-4o"))
//!   .build()
//!   .expect("agent instructions are set");
//!
//! assert_eq!(agent.model().map(AgentModel::id), Some("gpt-4o"));
//! ```
//!
//! [`crate::AgentModel`] selects a model by its platform-configured ID. If
//! omitted, the runtime can use the platform's default model. It can also
//! carry per-agent [`config::Generation`] settings and additional
//! provider-specific parameters; these are optional overrides, not a
//! separate model definition. When an agent is attached to a platform,
//! platform validation rejects an explicit model ID that is not configured.
//!
//! Builder setters for `domain_skills` and `platform_skills` replace the
//! entire corresponding list when called again; they do not append. Supply
//! the full list in one call. In contrast, a platform builder's `agent` and
//! `agents` methods append completed agents.
//!
//! # Creating the platform
//!
//! After building the agents, add them with [`AgentPlatformBuilder::agent`]
//! or [`AgentPlatformBuilder::agents`]. A model can be selected by ID, and
//! the platform default model is used when an agent has no explicit model.
//!
//! ```rust,no_run
//! use std::sync::Arc;
//!
//! use carisa_core::{AgentBuilder, AgentPlatformBuilder};
//! use carisa_core::memory::session::inmemory::InMemoryLogStorage;
//!
//! fn main() -> Result<(), Box<dyn std::error::Error>> {
//!   let agent = AgentBuilder::default()
//!     .instructions("Answer questions about the project.".to_owned())
//!     .model("gpt-4o")
//!     .build()?;
//!
//!   // Use from_env() instead to initialize from CARISA_ variables.
//!   let platform = AgentPlatformBuilder::from_file("runtime.json")?
//!     .agent(agent)
//!     .log_storage(Arc::new(InMemoryLogStorage::new()))
//!     .build()?;
//!
//!   assert_eq!(platform.agents().len(), 1);
//!   Ok(())
//! }
//! ```
//!
//! # Configuration sources
//!
//! [`AgentPlatformBuilder::from_file`] reads a JSON file and
//! [`AgentPlatformBuilder::from_env`] reads variables beginning with
//! `CARISA_`. Each is an independent initializer for a new builder; neither
//! automatically loads or overlays the other. After initialization, use the
//! builder methods to add or override values. There is no implicit priority
//! between a file and the environment.
//!
//! File and environment input support the `providers`, `http`, `models`, and
//! `defaults` sections. They do not create agents: a JSON `agents` property
//! and `CARISA_AGENTS` are ignored. Construct agents with
//! [`crate::AgentBuilder`] and attach them using the builder methods.
//!
//! Provider entries are keyed by their `name`, and model entries by their
//! `id`. Adding an entry with an existing key replaces that entire provider
//! or model; the fields inside the old and new values are not recursively
//! merged. New keys are appended. The builder's `http` and `defaults` values
//! are replaced as whole values when set again; `default_model` updates the
//! default model field. Calls to `agent` and `agents` append agents.
//!
//! For an all-Rust setup, start with `AgentPlatformBuilder::new(log_storage)`
//! or `AgentPlatformBuilder::default()` and set values with `providers`,
//! `http`, `models`, `default_model`, `agent`, and `log_storage`.
//! `build` then checks that providers and models exist, every model points
//! to a configured provider, default and agent model IDs exist, and HTTP
//! settings are available either platform-wide or on each provider.
//!
//! # JSON file format
//!
//! The root is an object. Provider and model collections are objects, and
//! each entry contains a complete serialized value. The map key is a label;
//! the provider's `name` and model's `id` fields determine identity. Values
//! use the field names and `snake_case` enum spellings shown below. See
//! [`config`] for the available fields and defaults.
//!
//! ```json
//! {
//!   "http": { "base_url": "https://api.openai.com/v1" },
//!   "providers": {
//!     "openai": {
//!       "name": "openai",
//!       "kind": "openai",
//!       "api_key": { "env": "OPENAI_API_KEY" },
//!       "retries": { "max_retries": 4, "backoff_ms": 250 }
//!     }
//!   },
//!   "models": {
//!     "gpt-4o": {
//!       "id": "gpt-4o",
//!       "provider": "openai",
//!       "generation": { "temperature": 0.2, "max_tokens": 2048 }
//!     }
//!   },
//!   "defaults": { "model": "gpt-4o" }
//! }
//! ```
//!
//! `api_key` is required by the provider value. A secret is represented as
//! one of `{"literal":"..."}`, `{"env":"VARIABLE_NAME"}`, or
//! `{"file":"/path/to/secret"}`. These variants describe the secret
//! source; the platform builder does not expand or read the referenced
//! environment variable or file. Avoid `literal` for real credentials in
//! checked-in configuration.
//!
//! # Environment variables
//!
//! Variable names use `CARISA_` followed by path components separated by
//! double underscores (`__`). Each component is lowercased, so the following
//! variables build the same shape as the JSON example:
//!
//! ```sh
//! CARISA_HTTP__BASE_URL=https://api.openai.com/v1
//! CARISA_PROVIDERS__OPENAI__NAME=openai
//! CARISA_PROVIDERS__OPENAI__KIND=openai
//! CARISA_PROVIDERS__OPENAI__API_KEY__ENV=OPENAI_API_KEY
//! CARISA_PROVIDERS__OPENAI__RETRIES__MAX_RETRIES=4
//! CARISA_MODELS__GPT-4O__ID=gpt-4o
//! CARISA_MODELS__GPT-4O__PROVIDER=openai
//! CARISA_MODELS__GPT-4O__GENERATION__TEMPERATURE=0.2
//! CARISA_DEFAULTS__MODEL=gpt-4o
//! ```
//!
//! Each value that is valid JSON is parsed as JSON: use `true`, `2048`, or
//! `0.2` for booleans and numbers. Other values remain strings. A complete
//! object can also be provided as a JSON-valued environment variable, for
//! example `CARISA_HTTP='{"base_url":"https://api.example"}'`.

pub mod config;
pub mod error;
pub mod platform;
pub mod provider;
pub mod secret;
pub mod types;

pub use error::PlatformError;
pub use platform::{AgentPlatform, AgentPlatformBuilder};
pub use provider::ProviderKind;
pub use secret::Secret;
pub use types::{Agent, AgentBuilder, AgentModel};
