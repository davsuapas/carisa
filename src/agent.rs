//! Agent runtime and type-state builders.
//!
//! This module exposes the public API used to construct runtime agents while
//! keeping the state-machine logic.
//!
//! # Building with the builder
//!
//! An agent without additional configuration can be built directly from the
//! initial state:
//!
//! ```
//! use carisa_core::AgentBuilder;
//!
//! let agent = AgentBuilder::default()
//!   .instructions("Help the user".to_owned())
//!   .build()
//!   .expect("all fields provided");
//! ```
//!
//! Domain skills and instructions can also be configured before building it:
//!
//! ```
//! use carisa_core::AgentBuilder;
//!
//! let agent = AgentBuilder::default()
//!   .instructions("Help the user".to_owned())
//!   .domain_skills(Vec::new())
//!   .build()
//!   .expect("all fields provided");
//! ```
//!
//! To include platform skills, drops standard platform skills,
//! start the transition with [`AgentBuilder::platform_skills`]:
//!
//! ```
//! use carisa_core::AgentBuilder;
//!
//! let agent = AgentBuilder::default()
//!   .instructions("Help the user".to_owned())
//!   .platform_skills(Vec::new())
//!   .build()
//!   .expect("all fields provided");
//! ```

pub mod types;

pub use types::AgentBuilder;
