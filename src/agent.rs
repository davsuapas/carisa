//! Agent runtime and type-state builders.
//!
//! This module exposes the public API used to construct runtime agents while
//! keeping the state-machine logic and filtering behaviour isolated in nested
//! modules.
//!
//! # Building with the builder
//!
//! An agent without additional configuration can be built directly from the
//! initial state:
//!
//! ```
//! use carisa_core::AgentBuilder;
//!
//! let agent = AgentBuilder::new().build();
//! ```
//!
//! Domain skills and instructions can also be configured before building it:
//!
//! ```
//! use carisa_core::AgentBuilder;
//!
//! let agent = AgentBuilder::new()
//!   .domain_skill(Vec::new())
//!   .instructions("Help the user".to_owned())
//!   .build();
//! ```
//!
//! To include platform skills, start the transition with
//! [`AgentBuilder::platform_skill`]:
//!
//! ```
//! use carisa_core::AgentBuilder;
//!
//! let agent = AgentBuilder::new()
//!   .platform_skill(Vec::new())
//!   .build();
//! ```
//!
//! The disable flow can exclude specific groups:
//!
//! ```
//! use carisa_core::AgentBuilder;
//!
//! let agent = AgentBuilder::new()
//!   .platform_skill(Vec::new())
//!   .disable()
//!   .grupo1()
//!   .grupo2()
//!   .done()
//!   .build();
//! ```
//!
//! All groups can also be disabled with [`DisableBuilder::all`]:
//!
//! ```
//! use carisa_core::AgentBuilder;
//!
//! let agent = AgentBuilder::new()
//!   .platform_skill(Vec::new())
//!   .disable()
//!   .all()
//!   .done()
//!   .build();
//! ```
//!
//! When a configuration is repeated, the last call for that configuration
//! always prevails. In this example, disabling `grupo1` is preserved, while
//! the final domain skills and instructions are those configured after
//! `done()`:
//!
//! ```
//! use carisa_core::AgentBuilder;
//! use carisa_core::{DomainSkillBuilder, PlatformSkillBuilder};
//!
//! # let domain = DomainSkillBuilder::default()
//! #   .id("domain".to_owned())
//! #   .title("Domain".to_owned())
//! #   .description("Domain skill".to_owned())
//! #   .instructions("Domain instructions".to_owned())
//! #   .version("1.0.0".to_owned())
//! #   .build()
//! #   .expect("valid domain skill");
//! # let platform = PlatformSkillBuilder::default()
//! #   .id("platform".to_owned())
//! #   .title("Platform".to_owned())
//! #   .description("Platform skill".to_owned())
//! #   .instructions("Platform instructions".to_owned())
//! #   .version("1.0.0".to_owned())
//! #   .always_load(false)
//! #   .build()
//! #   .expect("valid platform skill");
//!
//! let agent = AgentBuilder::new()
//!   .domain_skill(vec![domain.clone()])
//!   .platform_skill(vec![platform])
//!   .disable()
//!   .grupo1()
//!   .done()
//!   .domain_skill(vec![domain.clone()])
//!   .instructions("Other instructions".to_owned())
//!   .build();
//! ```
//!
//! Therefore, when [`AgentBuilder::domain_skill`] is called multiple times,
//! the list supplied in the last call is the one used by the agent.

pub mod builder;
pub mod disable;
pub mod filter;
pub mod runtime;

pub use builder::{
  AgentBuilder, AgentBuilderFinal, AgentBuilderWithPlatform, AllGroupsBuilder,
  DisableBuilder,
};
pub use runtime::AgentRuntime;
