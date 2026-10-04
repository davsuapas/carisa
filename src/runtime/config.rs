//! Typed values used by runtime configuration.
//!
//! These types are available under `runtime::config`; they are not
//! re-exported from `runtime` or the crate root. Their serde representation
//! uses the Rust field names in `snake_case`. A provider requires `name`,
//! `kind`, and `api_key`; a model requires `id` and `provider`; HTTP settings
//! require `base_url`. Other fields are optional or defaulted where noted on
//! each type. Provider and model identity during builder merges comes from
//! `name` and `id`, respectively, not from the JSON object's map key.
//!
//! ```json
//! {
//!   "http": {
//!     "base_url": "https://api.example",
//!     "headers": { "x-client": "carisa" },
//!     "options": { "timeout": 30 }
//!   },
//!   "providers": {
//!     "openai": {
//!       "name": "openai",
//!       "kind": "openai",
//!       "api_key": { "env": "OPENAI_API_KEY" },
//!       "tls": { "insecure_skip_verify": false }
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
//!         "additional_params": { "reasoning_effort": "high" }
//!       }
//!     }
//!   },
//!   "defaults": { "model": "gpt-4o" }
//! }
//! ```

use std::collections::HashMap;

use derive_builder::Builder;
use serde::{Deserialize, Serialize};

use super::provider::ProviderKind;
use super::secret::Secret;

/// Additional generation parameters.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Builder)]
#[builder(pattern = "owned")]
#[non_exhaustive]
pub struct Generation {
  /// Sampling temperature.
  #[builder(default, setter(into, strip_option))]
  temperature: Option<f64>,
  /// Maximum number of tokens to generate.
  #[builder(default, setter(into, strip_option))]
  max_tokens: Option<u32>,
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

  /// Additional model parameters keyed by name.
  pub const fn additional_params(&self) -> &HashMap<String, serde_json::Value> {
    &self.additional_params
  }
}

/// HTTP client options for a provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Builder)]
#[builder(pattern = "owned")]
#[non_exhaustive]
pub struct Http {
  /// Base URL used for HTTP requests.
  base_url: String,
  /// Custom request headers.
  #[builder(default)]
  #[serde(default)]
  headers: HashMap<String, String>,
  /// Additional options used by the provider.
  #[builder(default)]
  #[serde(default)]
  options: HashMap<String, serde_json::Value>,
}

impl Http {
  /// Creates an HTTP configuration with its required base URL.
  pub fn new(base_url: impl Into<String>) -> Self {
    Self {
      base_url: base_url.into(),
      headers: HashMap::new(),
      options: HashMap::new(),
    }
  }

  /// Returns the base URL for the provider.
  pub fn base_url(&self) -> &str {
    &self.base_url
  }

  /// Returns the custom request headers for the provider.
  pub const fn headers(&self) -> &HashMap<String, String> {
    &self.headers
  }

  /// Returns the additional options for the provider.
  pub const fn options(&self) -> &HashMap<String, serde_json::Value> {
    &self.options
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

/// A provider definition used by the runtime.
///
/// Its serialized `kind` is a stable lowercase identifier such as `openai`
/// or `openai_compatible`. `api_key` is required by this value's schema even
/// for provider kinds that do not use authentication. Optional provider HTTP
/// settings override the platform-wide HTTP settings for that provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Builder)]
#[builder(pattern = "owned")]
#[non_exhaustive]
pub struct Provider {
  /// Stable identifier for the provider.
  name: String,
  /// Provider family.
  kind: ProviderKind,
  /// Provider API key secret.
  api_key: Secret,
  /// HTTP options.
  #[builder(default, setter(into, strip_option))]
  http: Option<Http>,
  /// Retry policy.
  #[builder(default)]
  #[serde(default)]
  retries: Retries,
  /// TLS settings.
  #[builder(default, setter(into, strip_option))]
  tls: Option<Tls>,
}

impl Provider {
  /// Creates a provider with a name and kind.
  pub fn new(
    name: impl Into<String>,
    kind: ProviderKind,
    api_key: Secret,
  ) -> Self {
    Self {
      name: name.into(),
      kind,
      api_key,
      http: None,
      retries: Retries::default(),
      tls: None,
    }
  }

  /// Returns the provider name.
  pub fn name(&self) -> &str {
    &self.name
  }

  /// Returns the provider kind.
  pub const fn kind(&self) -> &ProviderKind {
    &self.kind
  }

  /// Returns the provider API key secret.
  pub const fn api_key(&self) -> &Secret {
    &self.api_key
  }

  /// Returns the optional HTTP configuration.
  pub const fn http(&self) -> Option<&Http> {
    self.http.as_ref()
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
  fn http_requires_base_url_during_deserialization() {
    let result = serde_json::from_str::<Http>(r#"{"headers":{}}"#);

    assert!(result.is_err());
  }

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
  fn provider_http_and_tls_are_optional() {
    let provider = Provider::new(
      "local",
      ProviderKind::Ollama,
      Secret::Literal("test-key".into()),
    );

    assert!(provider.http.is_none());
    assert!(provider.tls.is_none());
  }

  #[test]
  fn deserialized_provider_uses_retry_defaults_when_omitted() {
    let provider: Provider = serde_json::from_value(serde_json::json!({
      "name": "openai",
      "kind": "openai",
      "api_key": { "literal": "test-key" }
    }))
    .expect("provider has required credentials");

    assert_eq!(provider.retries, Retries::default());
    assert!(provider.http.is_none());
    assert!(provider.tls.is_none());
  }

  #[test]
  fn provider_requires_api_key_during_deserialization() {
    let result = serde_json::from_value::<Provider>(serde_json::json!({
      "name": "openai",
      "kind": "openai"
    }));

    assert!(result.is_err());
  }
}
