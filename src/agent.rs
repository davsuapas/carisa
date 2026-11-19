//! Agent definitions and agent platform configuration.
//!
//! Create agents with [`crate::AgentBuilder`], then hand them to
//! [`AgentPlatformBuilder`], which combines providers, the shared
//! [`Transport`], models, defaults, agents, and log storage.
//! [`AgentPlatformBuilder::build`] validates every reference and every
//! resolved model, then yields an [`AgentPlatform`]. Providers and models
//! come from a JSON file or from Rust; the environment contributes the file
//! path and credentials only.
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
//! [`crate::AgentModel`] selects a model by its platform-configured ID and
//! optionally overrides part of that model's [`config::Generation`] and its
//! additional parameters. Both overrides default to empty, so
//! `AgentModel::new("gpt-4o")` inherits everything. Omitting the model
//! entirely binds the agent to the platform default. When an agent is
//! attached to a platform, validation rejects an ID that is not configured.
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
//!   // With CARISA_CONFIG_PATH set, from_env_path() reads this file instead.
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
//! The contract is a JSON file, read by
//! [`AgentPlatformBuilder::from_file`], or named by `CARISA_CONFIG_PATH`
//! and read by [`AgentPlatformBuilder::from_env_path`]. The environment adds
//! nothing else: with no `CARISA_`-to-configuration mapping there is no
//! implicit layering, and each load yields a fresh builder that later method
//! calls add to or replace.
//!
//! Provider entries are keyed by their `name`, and model entries by their
//! `id`; a key that disagrees with the entry is an error, not a label.
//! Adding an entry with an existing key replaces that entire provider or
//! model; the fields inside the old and new values are not recursively
//! merged. New keys are appended. `transport` and `defaults` are replaced as
//! whole values when set again; `default_model` updates the default model
//! field. Calls to `agent` and `agents` append agents.
//!
//! For an all-Rust setup, start with `AgentPlatformBuilder::new(log_storage)`
//! or `AgentPlatformBuilder::default()` and set values with `providers`,
//! `transport`, `models`, `default_model`, `agent`, and `log_storage`.
//!
//! # Precedence
//!
//! [`AgentPlatform::resolve`] applies the platform's precedence once:
//!
//! 1. the agent's model ID, or `defaults.model` when the agent selects none;
//! 2. that model's provider, through the model's `provider` reference;
//! 3. the model's [`config::Generation`] with the agent's overrides laid over
//!    it field by field. `additional_params` merges by key with the agent
//!    winning, and a field the agent leaves unset is inherited.
//!
//! `build` walks every agent through that path, so a broken reference or an
//! option the model refuses fails at startup instead of on the first call.
//!
//! # JSON file format
//!
//! The root is an object that requires `schema_version: 1`. It may contain
//! `providers`, `transport`, `models`, and `defaults`; any other section is
//! an error. Provider and model collections are objects holding complete
//! values. See [`config`] for the available fields and defaults.
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "transport": { "timeout_ms": 60000 },
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
//! `kind` is a rig provider selection: a bare vendor when only one format is
//! registered for it, `vendor/format` otherwise (`"minimax/anthropic"`).
//! There is no `openai_compatible`: a gateway that reproduces the `OpenAI`
//! API is `"openai"` plus a provider `base_url`. `api_key` is required only when
//! the selection needs a credential. A secret is one of
//! `{"literal":"..."}`, `{"env":"VARIABLE_NAME"}`, or
//! `{"file":"/path/to/secret"}`; it is resolved when a client is built, not
//! when the file is read. Avoid `literal` for real credentials in
//! checked-in configuration.
//!
//! # Agents are Rust values
//!
//! Agents are defined in Rust, so per-agent overrides are not yet an
//! operator-editable part of the file: until [`crate::Agent`] carries an
//! `id` and the format admits an `agents` section, changing an agent means
//! rebuilding. From configuration, `agent.model == None` is the only way to
//! say "use the default model".
//!
//! # Where rig lives
//!
//! [`connect`] is the only module that imports `rig_core`. Provider identity
//! is [`ProviderId`], taken from rig's registry; the transport, the request,
//! and the catalog checks are built there. Upgrading or forking rig is a
//! change confined to that module.

pub mod config;
pub mod connect;
pub mod error;
pub mod platform;
pub mod provider;
pub mod resolve;
pub mod secret;
pub mod types;

pub use config::Transport;
pub use error::PlatformError;
pub use platform::{AgentPlatform, AgentPlatformBuilder};
pub use provider::ProviderId;
pub use resolve::ResolvedModel;
pub use secret::Secret;
pub use types::{Agent, AgentBuilder, AgentModel};
