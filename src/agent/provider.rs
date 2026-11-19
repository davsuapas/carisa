//! Provider identity.
//!
//! [`ProviderId`] comes from `rig-core`'s registry and is the only
//! provider identifier this crate accepts. It is re-exported here so
//! configuration and validation can name it without importing `rig_core`;
//! the `connect` module owns every other rig type.
//!
//! A [`ProviderId`] is a validated `(vendor, format)` pair. It deserializes
//! through `ProviderId::resolve`, so:
//!
//! - a bare vendor works only when this build registers exactly one format
//!   for it (`"openai"`, `"deepseek"`, `"ollama"`);
//! - a vendor registered in two families must be qualified
//!   (`"minimax/anthropic"`, `"zai/openai"`);
//! - unknown vendors, unknown formats, and unregistered pairs are rejected
//!   with the alternatives, never replaced by a default.
//!
//! There is no `openai_compatible` value: a gateway that reproduces the
//! `OpenAI` API is `kind: "openai"` plus a provider `base_url`.

pub use super::connect::ProviderId;
