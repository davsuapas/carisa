//! Typed configuration values for an agent platform.
//!
//! These values describe *what* to call, never *how* to reach it: the
//! transport, the credentials, and the request itself are built by the
//! `connect` module, the only place that knows about `rig-core`. Field names
//! are the JSON keys, in `snake_case`, so the serialized form is the contract
//! an operator edits.
//!
//! - [`Provider`] names a vendor selection through [`ProviderId`]
//!   (`"openai"`, `"minimax/anthropic"`) and may override the dialect's
//!   endpoint, headers, retries, and TLS settings.
//! - [`Transport`] holds the settings every provider shares: timeouts,
//!   proxy, default headers, and an optional caller identity for corporate
//!   gateways.
//! - [`Model`] binds a model identifier to a provider and carries the base
//!   [`Generation`] that agents inherit from.
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "transport": {
//!     "timeout_ms": 60000,
//!     "connect_timeout_ms": 5000,
//!     "default_headers": { "x-client": "carisa" }
//!   },
//!   "providers": {
//!     "openai": {
//!       "name": "openai",
//!       "kind": "openai",
//!       "api_key": { "env": "OPENAI_API_KEY" }
//!     }
//!   },
//!   "models": {
//!     "gpt-4o": {
//!       "id": "gpt-4o",
//!       "name": "GPT-4o",
//!       "provider": "openai",
//!       "generation": {
//!         "temperature": 0.2,
//!         "max_tokens": 2048,
//!         "top_p": 0.9,
//!         "additional_params": { "reasoning_effort": "high" }
//!       }
//!     }
//!   },
//!   "defaults": { "model": "gpt-4o" }
//! }
//! ```
//!
//! [`ProviderId`]: crate::agent::ProviderId

use std::collections::HashMap;

use derive_builder::Builder;
use serde::{Deserialize, Serialize};
use url::Url;

use super::provider::ProviderId;
use super::secret::Secret;

/// Generation parameters shared by a model and its agents.
///
/// Every field is optional: a model sets the baseline, and an agent's
/// [`AgentModel`](crate::agent::AgentModel) overrides only the fields it
/// sets. Unset fields fall through to the provider's default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Builder)]
#[builder(pattern = "owned")]
#[non_exhaustive]
pub struct Generation {
  /// Sampling temperature.
  #[builder(default, setter(strip_option))]
  temperature: Option<f64>,
  /// Maximum number of tokens to generate.
  #[builder(default, setter(strip_option))]
  max_tokens: Option<u32>,
  /// Nucleus sampling probability mass.
  #[builder(default, setter(strip_option))]
  top_p: Option<f64>,
  /// Sampling seed, for providers that offer best-effort determinism.
  #[builder(default, setter(strip_option))]
  seed: Option<u64>,
  /// Additional model parameters keyed by name.
  #[builder(default)]
  #[serde(default)]
  additional_params: HashMap<String, serde_json::Value>,
}

impl Generation {
  /// Creates a minimal generation config with default values.
  pub fn new() -> Self {
    Self::default()
  }

  /// Sampling temperature for the model.
  pub const fn temperature(&self) -> Option<f64> {
    self.temperature
  }

  /// Maximum number of tokens to generate.
  pub const fn max_tokens(&self) -> Option<u32> {
    self.max_tokens
  }

  /// Nucleus sampling probability mass.
  pub const fn top_p(&self) -> Option<f64> {
    self.top_p
  }

  /// Sampling seed.
  pub const fn seed(&self) -> Option<u64> {
    self.seed
  }

  /// Additional model parameters keyed by name.
  pub const fn additional_params(&self) -> &HashMap<String, serde_json::Value> {
    &self.additional_params
  }

  /// Overlays `over` on this value: its set scalars win, and its additional
  /// parameters are merged by key.
  pub(crate) fn overlay(&mut self, over: &Self) {
    if over.temperature.is_some() {
      self.temperature = over.temperature;
    }
    if over.max_tokens.is_some() {
      self.max_tokens = over.max_tokens;
    }
    if over.top_p.is_some() {
      self.top_p = over.top_p;
    }
    if over.seed.is_some() {
      self.seed = over.seed;
    }
    self.overlay_params(&over.additional_params);
  }

  /// Merges `params` into the additional parameters, `params` winning.
  pub(crate) fn overlay_params(
    &mut self,
    params: &HashMap<String, serde_json::Value>,
  ) {
    self.additional_params.extend(
      params
        .iter()
        .map(|(key, value)| (key.clone(), value.clone())),
    );
  }
}

/// Identity a corporate gateway expects on every request.
///
/// It is valid only on a platform whose providers all speak the `OpenAI`
/// format, because that is the only family whose configuration carries one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Builder)]
#[builder(pattern = "owned")]
#[non_exhaustive]
pub struct CallerIdentity {
  /// Value of the gateway's `originator` header.
  originator: String,
  /// Value of the `user-agent` header, defaulting to the crate name and
  /// version.
  #[builder(default, setter(into, strip_option))]
  user_agent: Option<String>,
}

impl CallerIdentity {
  /// Creates an identity with the required originator.
  pub fn new(originator: impl Into<String>) -> Self {
    Self {
      originator: originator.into(),
      user_agent: None,
    }
  }

  /// Returns the `originator` value.
  pub fn originator(&self) -> &str {
    &self.originator
  }

  /// Returns the optional `user-agent` override.
  pub const fn user_agent(&self) -> Option<&String> {
    self.user_agent.as_ref()
  }
}

/// HTTP settings shared by every provider.
///
/// Each provider gets its own client built from these settings, so a proxy
/// or a timeout applies everywhere without repeating it per provider.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Builder)]
#[builder(pattern = "owned")]
#[serde(default)]
#[non_exhaustive]
pub struct Transport {
  /// Total time allowed for one request, in milliseconds.
  #[builder(default, setter(strip_option))]
  timeout_ms: Option<u64>,
  /// Time allowed to establish a connection, in milliseconds.
  #[builder(default, setter(strip_option))]
  connect_timeout_ms: Option<u64>,
  /// Proxy URL every provider routes through.
  #[builder(default, setter(strip_option))]
  proxy: Option<Url>,
  /// Headers added to every provider's requests.
  #[builder(default)]
  default_headers: HashMap<String, String>,
  /// Caller identity the gateway requires.
  #[builder(default, setter(strip_option))]
  identity: Option<CallerIdentity>,
}

impl Transport {
  /// Creates a transport with no settings.
  pub fn new() -> Self {
    Self::default()
  }

  /// Returns the request timeout in milliseconds.
  pub const fn timeout_ms(&self) -> Option<u64> {
    self.timeout_ms
  }

  /// Returns the connection timeout in milliseconds.
  pub const fn connect_timeout_ms(&self) -> Option<u64> {
    self.connect_timeout_ms
  }

  /// Returns the proxy URL.
  pub const fn proxy(&self) -> Option<&Url> {
    self.proxy.as_ref()
  }

  /// Returns the headers added to every request.
  pub const fn default_headers(&self) -> &HashMap<String, String> {
    &self.default_headers
  }

  /// Returns the configured caller identity.
  pub const fn identity(&self) -> Option<&CallerIdentity> {
    self.identity.as_ref()
  }
}

/// Retry policy details used when a provider call fails.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Builder)]
#[builder(pattern = "owned")]
#[serde(default)]
#[non_exhaustive]
pub struct Retries {
  /// Total retry count.
  #[builder(default = "3")]
  max_retries: u32,
  /// Initial backoff delay in milliseconds.
  #[builder(default = "500")]
  backoff_ms: u64,
}

impl Default for Retries {
  fn default() -> Self {
    Self {
      max_retries: 3,
      backoff_ms: 500,
    }
  }
}

impl Retries {
  /// Creates a new retry policy with default values.
  pub fn new() -> Self {
    Self::default()
  }

  /// Returns the maximum number of retries.
  pub const fn max_retries(&self) -> u32 {
    self.max_retries
  }

  /// Returns the initial backoff delay in milliseconds.
  pub const fn backoff_ms(&self) -> u64 {
    self.backoff_ms
  }
}

/// TLS configuration for provider connections.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Builder)]
#[builder(pattern = "owned")]
#[non_exhaustive]
pub struct Tls {
  /// Whether insecure TLS certificates are accepted.
  #[builder(default)]
  insecure_skip_verify: bool,
  /// Optional custom CA bundle path.
  #[builder(default, setter(into, strip_option))]
  ca_file: Option<String>,
}

impl Tls {
  /// Creates a new TLS configuration with default values.
  pub fn new() -> Self {
    Self::default()
  }

  /// Returns whether insecure TLS certificates are accepted.
  pub const fn insecure_skip_verify(&self) -> bool {
    self.insecure_skip_verify
  }

  /// Returns the optional custom CA bundle path.
  pub const fn ca_file(&self) -> Option<&String> {
    self.ca_file.as_ref()
  }
}

/// Default runtime settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Builder)]
#[builder(pattern = "owned")]
#[non_exhaustive]
pub struct Defaults {
  /// Default model chosen when an agent does not specify one.
  #[builder(default, setter(into, strip_option))]
  model: Option<String>,
}

impl Defaults {
  /// Creates a new defaults value.
  pub fn new() -> Self {
    Self::default()
  }

  /// Returns the default model identifier.
  pub const fn model(&self) -> Option<&String> {
    self.model.as_ref()
  }

  pub(crate) fn with_model(mut self, model: impl Into<String>) -> Self {
    self.model = Some(model.into());
    self
  }
}

/// A model definition for a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Builder)]
#[builder(pattern = "owned")]
#[non_exhaustive]
pub struct Model {
  /// Model identifier.
  id: String,
  /// Human-readable display name.
  #[builder(default, setter(into, strip_option))]
  name: Option<String>,
  /// Provider key for this model.
  provider: String,
  /// Base model configuration.
  #[builder(default)]
  #[serde(default)]
  generation: Generation,
}

impl Model {
  /// Creates a model with the required fields.
  pub fn new(id: impl Into<String>, provider: impl Into<String>) -> Self {
    Self {
      id: id.into(),
      name: None,
      provider: provider.into(),
      generation: Generation::default(),
    }
  }

  /// Returns the model identifier.
  pub fn id(&self) -> &str {
    &self.id
  }

  /// Returns the optional human-readable display name.
  pub const fn name(&self) -> Option<&String> {
    self.name.as_ref()
  }

  /// Returns the provider key for this model.
  pub fn provider(&self) -> &str {
    &self.provider
  }

  /// Returns the base model configuration.
  pub const fn generation(&self) -> &Generation {
    &self.generation
  }
}

/// A provider definition.
///
/// `kind` is a `rig-core` provider selection: a bare vendor (`"openai"`)
/// when this build registers exactly one format for it, or `vendor/format`
/// (`"minimax/anthropic"`) when it registers more than one. Unknown,
/// ambiguous, and unregistered values are rejected with their alternatives,
/// never silently replaced.
///
/// `api_key` is required only when the selection requires a credential, and
/// is ignored otherwise. `base_url` overrides the dialect's endpoint: an
/// OpenAI-compatible gateway is expressed as `"openai"` plus `base_url`.
/// `headers` are added on top of [`Transport::default_headers`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Builder)]
#[builder(pattern = "owned")]
#[non_exhaustive]
pub struct Provider {
  /// Stable identifier for the provider.
  name: String,
  /// Provider selection.
  kind: ProviderId,
  /// Credential source, when the selection needs one.
  #[builder(default, setter(into, strip_option))]
  api_key: Option<Secret>,
  /// Endpoint override for this provider.
  #[builder(default, setter(strip_option))]
  base_url: Option<Url>,
  /// Headers added to this provider's requests.
  #[builder(default)]
  #[serde(default)]
  headers: HashMap<String, String>,
  /// Retry policy.
  #[builder(default)]
  #[serde(default)]
  retries: Retries,
  /// TLS settings.
  #[builder(default, setter(strip_option))]
  tls: Option<Tls>,
}

impl Provider {
  /// Creates a provider with the required fields.
  pub fn new(
    name: impl Into<String>,
    kind: ProviderId,
    api_key: Option<Secret>,
  ) -> Self {
    Self {
      name: name.into(),
      kind,
      api_key,
      base_url: None,
      headers: HashMap::new(),
      retries: Retries::default(),
      tls: None,
    }
  }

  /// Returns the provider name.
  pub fn name(&self) -> &str {
    &self.name
  }

  /// Returns the provider selection.
  pub const fn kind(&self) -> &ProviderId {
    &self.kind
  }

  /// Returns the credential source, when one is configured.
  pub const fn api_key(&self) -> Option<&Secret> {
    self.api_key.as_ref()
  }

  /// Returns the endpoint override.
  pub const fn base_url(&self) -> Option<&Url> {
    self.base_url.as_ref()
  }

  /// Returns the headers added to this provider's requests.
  pub const fn headers(&self) -> &HashMap<String, String> {
    &self.headers
  }

  /// Returns the retry policy.
  pub const fn retries(&self) -> &Retries {
    &self.retries
  }

  /// Returns the optional TLS configuration.
  pub const fn tls(&self) -> Option<&Tls> {
    self.tls.as_ref()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn retries_default_to_three_attempts_and_half_second_backoff() {
    assert_eq!(
      Retries::default(),
      Retries {
        max_retries: 3,
        backoff_ms: 500
      },
    );
    assert_eq!(
      serde_json::from_str::<Retries>("{}")
        .expect("empty retry policy uses defaults"),
      Retries::default(),
    );
  }

  #[test]
  fn provider_network_settings_are_optional() {
    let provider = Provider::new(
      "local",
      ProviderId::resolve("ollama/openai").expect("the pair is registered"),
      None,
    );

    assert!(provider.api_key().is_none());
    assert!(provider.base_url().is_none());
    assert!(provider.headers().is_empty());
    assert!(provider.tls().is_none());
  }

  #[test]
  fn deserialized_provider_uses_retry_defaults_when_omitted() {
    let provider: Provider = serde_json::from_value(serde_json::json!({
      "name": "openai",
      "kind": "openai",
      "api_key": { "literal": "test-key" }
    }))
    .expect("provider has the required fields");

    assert_eq!(provider.retries, Retries::default());
    assert!(provider.base_url.is_none());
    assert!(provider.headers.is_empty());
    assert!(provider.tls.is_none());
  }

  #[test]
  fn provider_api_key_is_optional_during_deserialization() {
    let provider: Provider = serde_json::from_value(serde_json::json!({
      "name": "local",
      "kind": "ollama/openai"
    }))
    .expect("a local provider needs no credential");

    assert!(provider.api_key().is_none());
  }

  #[test]
  fn invalid_base_url_is_rejected_during_deserialization() {
    let result = serde_json::from_value::<Provider>(serde_json::json!({
      "name": "gateway",
      "kind": "openai",
      "base_url": "not a url"
    }));

    assert!(result.is_err());
  }

  #[test]
  fn ambiguous_kind_is_rejected_with_qualified_alternatives() {
    let error = serde_json::from_value::<Provider>(serde_json::json!({
      "name": "gateway",
      "kind": "minimax"
    }))
    .expect_err("minimax registers more than one format");

    assert!(error.to_string().contains("minimax/openai"));
    assert!(error.to_string().contains("minimax/anthropic"));
  }

  #[test]
  fn generation_overlay_keeps_the_base_and_lets_the_override_win() {
    let mut base = GenerationBuilder::default()
      .temperature(0.2)
      .max_tokens(2048_u32)
      .additional_params(HashMap::from([(
        "reasoning_effort".to_owned(),
        serde_json::json!("low"),
      )]))
      .build()
      .expect("no required fields");
    let over = GenerationBuilder::default()
      .temperature(0.9)
      .top_p(0.5)
      .additional_params(HashMap::from([(
        "reasoning_effort".to_owned(),
        serde_json::json!("high"),
      )]))
      .build()
      .expect("no required fields");

    base.overlay(&over);

    assert_eq!(base.temperature(), Some(0.9));
    assert_eq!(base.max_tokens(), Some(2048));
    assert_eq!(base.top_p(), Some(0.5));
    assert_eq!(base.seed(), None);
    assert_eq!(
      base.additional_params().get("reasoning_effort"),
      Some(&serde_json::json!("high"))
    );
  }
}
