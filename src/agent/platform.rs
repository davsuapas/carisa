//! Builder and validated agent platform.
//!
//! The builder gathers providers, the shared [`Transport`], models, defaults,
//! agents, and log storage, then validates them into an
//! [`AgentPlatform`]. Configuration may come from a
//! JSON file; agents and log storage are always supplied in Rust. See the
//! parent [`agent`](super) module for the file format and the resolution
//! rules.

use std::{fs, path::Path, sync::Arc};

use derive_builder::Builder;

use crate::memory::session::LogStorage;

use super::{
  config::{Defaults, Generation, Model, Provider, Transport},
  connect,
  error::PlatformError,
  resolve::ResolvedModel,
};

/// The only schema version this build reads.
const SCHEMA_VERSION: u64 = 1;

/// Largest accepted retry count.
const MAX_RETRIES: u32 = 10;

/// Inclusive bounds for a sampling temperature.
const TEMPERATURE_RANGE: std::ops::RangeInclusive<f64> = 0.0..=2.0;

/// Inclusive bounds for nucleus sampling.
const TOP_P_RANGE: std::ops::RangeInclusive<f64> = 0.0..=1.0;

/// Configuration to run agents at runtime.
#[derive(Builder, Clone)]
#[builder(
  pattern = "owned",
  build_fn(name = "build_inner", private),
  derive(Clone)
)]
pub struct AgentPlatform {
  /// Configured providers.
  #[builder(default, setter(name = "provider_values"))]
  providers: Vec<Provider>,
  /// HTTP settings shared by every provider.
  #[builder(default, setter(strip_option))]
  transport: Option<Transport>,
  /// Configured models.
  #[builder(default, setter(name = "model_values"))]
  models: Vec<Model>,
  /// Platform defaults.
  #[builder(default)]
  defaults: Defaults,
  /// Configured agents.
  #[builder(default, setter(name = "agent_values"))]
  agents: Vec<crate::agent::Agent>,
  /// Required active log storage backend.
  #[expect(dead_code)]
  log_storage: Arc<dyn LogStorage>,
}

impl std::fmt::Debug for AgentPlatform {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("AgentPlatform")
      .field("providers", &self.providers)
      .field("transport", &self.transport)
      .field("models", &self.models)
      .field("defaults", &self.defaults)
      .field("agents", &self.agents)
      .field("log_storage", &"Arc<dyn LogStorage>")
      .finish()
  }
}

impl std::fmt::Debug for AgentPlatformBuilder {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("AgentPlatformBuilder")
      .field("providers", &self.providers)
      .field("transport", &self.transport)
      .field("models", &self.models)
      .field("defaults", &self.defaults)
      .field("agents", &self.agents)
      .field("log_storage", &"Arc<dyn LogStorage>")
      .finish()
  }
}

impl AgentPlatformBuilder {
  /// Creates a builder with the required log storage backend.
  #[must_use]
  pub fn new(log_storage: Arc<dyn LogStorage>) -> Self {
    Self::default().log_storage(log_storage)
  }

  /// Adds or replaces providers by name.
  ///
  /// Providers with new names are appended. When a name already exists, the
  /// incoming provider replaces the existing provider completely.
  #[must_use]
  pub fn providers(
    mut self,
    providers: impl IntoIterator<Item = Provider>,
  ) -> Self {
    let configured = self.providers.get_or_insert_with(Vec::new);
    for provider in providers {
      upsert_by(configured, provider, |item| item.name());
    }
    self
  }

  /// Adds or replaces models by identifier.
  ///
  /// Models with new identifiers are appended. When an identifier already
  /// exists, the incoming model replaces the existing model completely.
  #[must_use]
  pub fn models(mut self, models: impl IntoIterator<Item = Model>) -> Self {
    let configured = self.models.get_or_insert_with(Vec::new);
    for model in models {
      upsert_by(configured, model, |item| item.id());
    }
    self
  }

  /// Loads provider, transport, model, and default settings from a JSON file.
  ///
  /// The root object requires `schema_version: 1` and may contain
  /// `providers`, `transport`, `models`, and `defaults`. Unknown sections
  /// are rejected rather than ignored, so a typo cannot silently produce a
  /// default. Provider and model collections are objects whose keys must
  /// match each entry's `name` and `id`. This creates a fresh builder; use
  /// its methods to add agents or override loaded values.
  ///
  /// Entries are keyed for replacement by that same `name` or `id`.
  /// Replacing an existing key replaces the complete value, not just the
  /// fields present in a later source.
  ///
  /// # Errors
  ///
  /// Returns an error when the file cannot be opened or parsed, when a
  /// section or map key is unknown, or when `schema_version` is missing or
  /// unsupported.
  pub fn from_file(path: impl AsRef<Path>) -> Result<Self, PlatformError> {
    let path = path.as_ref();
    let content =
      fs::read_to_string(path).map_err(|source| PlatformError::Io {
        path: path.to_string_lossy().to_string(),
        source,
      })?;
    let value: serde_json::Value =
      serde_json::from_str(&content).map_err(|source| PlatformError::Json {
        path: path.to_string_lossy().to_string(),
        source,
      })?;

    let mut builder = Self::default();
    builder.merge_json_value(value)?;
    Ok(builder)
  }

  /// Loads configuration from the file named by `CARISA_CONFIG_PATH`.
  ///
  /// The environment carries where the configuration lives and, through
  /// [`Secret::Env`](crate::agent::Secret::Env), where credentials live;
  /// everything else is in the file.
  ///
  /// # Errors
  ///
  /// [`PlatformError::MissingEnv`] when `CARISA_CONFIG_PATH` is not set, and
  /// any error [`Self::from_file`] reports.
  pub fn from_env_path() -> Result<Self, PlatformError> {
    let variable = "CARISA_CONFIG_PATH";
    let path =
      std::env::var(variable).map_err(|_| PlatformError::MissingEnv {
        variable: variable.to_owned(),
      })?;
    Self::from_file(path)
  }

  /// Sets the default model identifier.
  #[must_use]
  pub fn default_model(mut self, id: impl Into<String>) -> Self {
    self.defaults =
      Some(self.defaults.take().unwrap_or_default().with_model(id));
    self
  }

  /// Appends one agent to the platform.
  #[must_use]
  pub fn agent(mut self, agent: crate::agent::Agent) -> Self {
    self.agents.get_or_insert_with(Vec::new).push(agent);
    self
  }

  /// Appends multiple agents to the platform.
  #[must_use]
  pub fn agents(
    mut self,
    agents: impl IntoIterator<Item = crate::agent::Agent>,
  ) -> Self {
    self.agents.get_or_insert_with(Vec::new).extend(agents);
    self
  }

  /// Builds and validates the resolved platform.
  ///
  /// # Errors
  ///
  /// Returns an error when required data is missing or a reference points
  /// to a missing target.
  pub fn build(self) -> Result<AgentPlatform, PlatformError> {
    let platform = self
      .build_inner()
      .map_err(|error| PlatformError::invalid("platform", error.to_string()))?;
    platform.validate()?;
    Ok(platform)
  }

  fn merge_json_value(
    &mut self,
    value: serde_json::Value,
  ) -> Result<(), PlatformError> {
    let serde_json::Value::Object(mut sections) = value else {
      return Err(PlatformError::invalid(
        "$",
        "the configuration root must be an object",
      ));
    };

    let version = sections.remove("schema_version").ok_or_else(|| {
      PlatformError::invalid("schema_version", "is required and must be 1")
    })?;
    let version = version.as_u64().ok_or_else(|| {
      PlatformError::invalid("schema_version", "must be a positive integer")
    })?;
    if version != SCHEMA_VERSION {
      return Err(PlatformError::SchemaVersion { version });
    }

    for (section, value) in sections {
      match section.as_str() {
        "providers" => self.merge_providers(value)?,
        "transport" => {
          self.transport =
            Some(Some(serde_json::from_value::<Transport>(value).map_err(
              |source| PlatformError::Json {
                path: "transport".to_owned(),
                source,
              },
            )?));
        }
        "models" => self.merge_models(value)?,
        "defaults" => {
          self.defaults =
            Some(serde_json::from_value::<Defaults>(value).map_err(
              |source| PlatformError::Json {
                path: "defaults".to_owned(),
                source,
              },
            )?);
        }
        unknown => {
          return Err(PlatformError::invalid(
            unknown,
            "unknown configuration section",
          ));
        }
      }
    }

    Ok(())
  }

  /// Merges the `providers` section, rejecting keys that do not match the
  /// provider's `name` and kinds this build cannot configure.
  fn merge_providers(
    &mut self,
    value: serde_json::Value,
  ) -> Result<(), PlatformError> {
    let serde_json::Value::Object(entries) = value else {
      return Err(PlatformError::invalid("providers", "must be an object"));
    };

    for (key, entry) in entries {
      let path = format!("providers.{key}");
      if let Some(kind) = entry.get("kind").and_then(serde_json::Value::as_str)
      {
        connect::resolve_provider_kind(&format!("{path}.kind"), kind)?;
      }
      let provider =
        serde_json::from_value::<Provider>(entry).map_err(|source| {
          PlatformError::Json {
            path: path.clone(),
            source,
          }
        })?;
      if provider.name() != key {
        return Err(PlatformError::invalid(
          path,
          "map key must match the provider's `name`",
        ));
      }
      upsert_by(
        self.providers.get_or_insert_with(Vec::new),
        provider,
        |item| item.name(),
      );
    }

    Ok(())
  }

  /// Merges the `models` section, rejecting keys that do not match the
  /// model's `id`.
  fn merge_models(
    &mut self,
    value: serde_json::Value,
  ) -> Result<(), PlatformError> {
    let serde_json::Value::Object(entries) = value else {
      return Err(PlatformError::invalid("models", "must be an object"));
    };

    for (key, entry) in entries {
      let path = format!("models.{key}");
      let model = serde_json::from_value::<Model>(entry).map_err(|source| {
        PlatformError::Json {
          path: path.clone(),
          source,
        }
      })?;
      if model.id() != key {
        return Err(PlatformError::invalid(
          path,
          "map key must match the model's `id`",
        ));
      }
      upsert_by(self.models.get_or_insert_with(Vec::new), model, |item| {
        item.id()
      });
    }

    Ok(())
  }
}

impl AgentPlatform {
  /// Creates a platform from resolved configuration values.
  pub fn new(
    providers: Vec<Provider>,
    transport: Option<Transport>,
    models: Vec<Model>,
    defaults: Defaults,
    agents: Vec<crate::agent::Agent>,
    log_storage: Arc<dyn LogStorage>,
  ) -> Self {
    Self {
      providers,
      transport,
      models,
      defaults,
      agents,
      log_storage,
    }
  }

  /// Returns the configured providers.
  pub fn providers(&self) -> &[Provider] {
    &self.providers
  }

  /// Returns the HTTP settings shared by every provider.
  pub const fn transport(&self) -> Option<&Transport> {
    self.transport.as_ref()
  }

  /// Returns the configured models.
  pub fn models(&self) -> &[Model] {
    &self.models
  }

  /// Returns the platform defaults.
  pub const fn defaults(&self) -> &Defaults {
    &self.defaults
  }

  /// Returns the configured agents.
  pub fn agents(&self) -> &[crate::Agent] {
    &self.agents
  }

  /// Validates the platform without building a client.
  ///
  /// Providers must exist and be configurable by this build; each one must
  /// carry a credential exactly when its selection requires one. Models must
  /// point at a configured provider, and every agent must resolve through
  /// [`Self::resolve`], so precedence is checked at startup rather than on
  /// the first call. Resolved models the catalog knows are checked against
  /// it; models it does not know are accepted as local or gateway-specific.
  ///
  /// # Errors
  ///
  /// [`PlatformError::Invalid`] at the offending path, or
  /// [`PlatformError::SchemaVersion`] for an unsupported schema.
  pub(crate) fn validate(&self) -> Result<(), PlatformError> {
    if self.providers.is_empty() {
      return Err(PlatformError::invalid(
        "providers",
        "at least one provider is required",
      ));
    }

    if self.models.is_empty() {
      return Err(PlatformError::invalid(
        "models",
        "at least one model is required",
      ));
    }

    if let Some(transport) = &self.transport {
      validate_transport(transport)?;
    }

    if self
      .transport
      .as_ref()
      .is_some_and(|transport| transport.identity().is_some())
      && !self
        .providers
        .iter()
        .all(|provider| connect::speaks_openai(provider.kind()))
    {
      return Err(PlatformError::invalid(
        "transport.identity",
        "caller identity requires every provider to speak the OpenAI format",
      ));
    }

    for provider in &self.providers {
      validate_provider(provider)?;
    }

    for model in &self.models {
      let Some(provider) = self
        .providers
        .iter()
        .find(|provider| provider.name() == model.provider())
      else {
        return Err(PlatformError::invalid(
          format!("models.{}.provider", model.id()),
          "referenced provider does not exist",
        ));
      };
      validate_generation(
        &format!("models.{}", model.id()),
        model.generation(),
      )?;
      let resolved =
        ResolvedModel::new(provider, model, model.generation().clone());
      connect::validate_against_catalog(&resolved)?;
    }

    if let Some(model_id) = self.defaults.model()
      && !self.models.iter().any(|model| model.id() == model_id)
    {
      return Err(PlatformError::invalid(
        "defaults.model",
        "default model does not exist",
      ));
    }

    for (idx, agent) in self.agents.iter().enumerate() {
      let resolved =
        self.resolve_at(agent, &format!("agents[{idx}].model.id"))?;
      validate_generation(
        &format!("agents[{idx}].model.generation"),
        resolved.generation(),
      )?;
      connect::validate_against_catalog(&resolved)?;
    }

    Ok(())
  }
}

/// Validates the shared transport settings.
fn validate_transport(transport: &Transport) -> Result<(), PlatformError> {
  validate_timeout("transport.timeout_ms", transport.timeout_ms())?;
  validate_timeout(
    "transport.connect_timeout_ms",
    transport.connect_timeout_ms(),
  )?;
  connect::validate_headers(
    "transport.default_headers",
    transport.default_headers(),
  )?;
  Ok(())
}

/// Validates one provider's identity, credential, retries, and headers.
fn validate_provider(provider: &Provider) -> Result<(), PlatformError> {
  let path = format!("providers.{}", provider.name());

  if !provider.kind().is_registered() {
    return Err(PlatformError::invalid(
      format!("{path}.kind"),
      format!("`{}` is not configurable by this build", provider.kind()),
    ));
  }

  if provider.kind().requires_credential() && provider.api_key().is_none() {
    return Err(PlatformError::invalid(
      format!("{path}.api_key"),
      format!("`{}` requires a credential", provider.kind()),
    ));
  }

  if provider.retries().max_retries() > MAX_RETRIES {
    return Err(PlatformError::invalid(
      format!("{path}.retries.max_retries"),
      format!("must be at most {MAX_RETRIES}"),
    ));
  }

  if provider.retries().backoff_ms() == 0 {
    return Err(PlatformError::invalid(
      format!("{path}.retries.backoff_ms"),
      "must be greater than zero",
    ));
  }

  connect::validate_headers(&format!("{path}.headers"), provider.headers())?;
  Ok(())
}

/// Validates a timeout that must be positive when set.
fn validate_timeout(
  path: &str,
  value: Option<u64>,
) -> Result<(), PlatformError> {
  match value {
    Some(0) => Err(PlatformError::invalid(path, "must be greater than zero")),
    _ => Ok(()),
  }
}

/// Validates sampling bounds for a generation block.
fn validate_generation(
  path: &str,
  generation: &Generation,
) -> Result<(), PlatformError> {
  if let Some(temperature) = generation.temperature()
    && !TEMPERATURE_RANGE.contains(&temperature)
  {
    return Err(PlatformError::invalid(
      format!("{path}.temperature"),
      "must be within 0.0..=2.0",
    ));
  }

  if let Some(top_p) = generation.top_p()
    && !TOP_P_RANGE.contains(&top_p)
  {
    return Err(PlatformError::invalid(
      format!("{path}.top_p"),
      "must be within 0.0..=1.0",
    ));
  }

  Ok(())
}

fn upsert_by<T>(items: &mut Vec<T>, item: T, key: impl Fn(&T) -> &str) {
  if let Some(current) =
    items.iter_mut().find(|current| key(current) == key(&item))
  {
    *current = item;
  } else {
    items.push(item);
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    agent::{
      ProviderId, Secret,
      config::{
        CallerIdentity, GenerationBuilder, ModelBuilder, ProviderBuilder,
        Retries, RetriesBuilder, TlsBuilder, TransportBuilder,
      },
    },
    memory::session::inmemory::InMemoryLogStorage,
  };
  use rstest::rstest;

  fn provider(name: &str) -> Provider {
    provider_of(name, "openai")
  }

  fn provider_of(name: &str, kind: &str) -> Provider {
    ProviderBuilder::default()
      .name(name.to_owned())
      .kind(ProviderId::resolve(kind).expect("kind is registered"))
      .api_key(Secret::Literal("test-api-key".to_owned()))
      .build()
      .expect("provider builder has all required fields")
  }

  fn valid_builder() -> AgentPlatformBuilder {
    AgentPlatformBuilder::default()
      .providers(vec![provider("openai"), provider("backup")])
      .models(vec![Model::new("gpt-4o", "openai")])
      .log_storage(Arc::new(InMemoryLogStorage::new()))
  }

  fn assert_serde_round_trip<T>(value: &T)
  where
    T: serde::Serialize
      + for<'de> serde::Deserialize<'de>
      + PartialEq
      + std::fmt::Debug,
  {
    let serialized = serde_json::to_value(value).expect("value serializes");
    let deserialized =
      serde_json::from_value::<T>(serialized).expect("value deserializes");

    assert_eq!(&deserialized, value);
  }

  fn load_file_config(content: &str) -> AgentPlatformBuilder {
    let path = std::env::temp_dir()
      .join(format!("carisa-platform-{}.json", ulid::Ulid::generate()));
    fs::write(&path, content).expect("temporary config is written");
    let result = AgentPlatformBuilder::from_file(&path);
    fs::remove_file(path).expect("temporary config is removed");

    result.expect("configuration file loads")
  }

  #[test]
  fn builder_accepts_provider_vectors_and_defaults() {
    let platform =
      AgentPlatformBuilder::new(Arc::new(InMemoryLogStorage::new()))
        .providers(vec![provider("openai"), provider("backup")])
        .models(vec![Model::new("gpt-4o", "openai")])
        .default_model("gpt-4o")
        .build()
        .expect("valid default config");

    assert_eq!(platform.providers.len(), 2);
    assert_eq!(
      platform.defaults.model().map(String::as_str),
      Some("gpt-4o")
    );
    assert_eq!(
      platform.providers.first().map(Provider::name),
      Some("openai")
    );
  }

  #[test]
  fn agent_model_identifier_must_exist() {
    let agent = crate::agent::AgentBuilder::default()
      .instructions("Instructions".to_owned())
      .model("missing-model")
      .build()
      .expect("instructions provided");
    let error = valid_builder().agent(agent).build().err();

    assert!(error.is_some_and(|error| {
      error.to_string().contains("agents[0].model.id")
    }));
  }

  #[test]
  fn agent_model_identifier_matches_model_id() {
    let agent = crate::agent::AgentBuilder::default()
      .instructions("Instructions".to_owned())
      .model("model-id")
      .build()
      .expect("instructions provided");
    let platform = AgentPlatform::new(
      vec![provider("openai"), provider("backup")],
      None,
      vec![Model::new("model-id", "openai")],
      Defaults::default(),
      vec![agent],
      Arc::new(InMemoryLogStorage::new()),
    );

    assert!(platform.validate().is_ok());
  }

  #[rstest]
  #[case("Providers are empty.Invalid", "providers")]
  #[case("Models are empty.Invalid", "models")]
  #[case("Transport timeout is zero.Invalid", "transport.timeout_ms")]
  #[case("Temperature is out of range.Invalid", "models.gpt-4o.temperature")]
  #[case("Top p is out of range.Invalid", "models.gpt-4o.top_p")]
  #[case(
    "Retries exceed the maximum.Invalid",
    "providers.openai.retries.max_retries"
  )]
  #[case("Provider header is malformed.Invalid", "providers.openai.headers")]
  #[case("Provider credential is missing.Invalid", "providers.openai.api_key")]
  #[case("Identity with a non-OpenAI provider.Invalid", "transport.identity")]
  #[case("Model provider is unknown.Invalid", "models.gpt-4o.provider")]
  #[case("Default model is unknown.Invalid", "defaults.model")]
  #[case("Agent model is unknown.Invalid", "agents[0].model.id")]
  fn validate_rejects_each_invalid_platform_case(
    #[case] case_id: &str,
    #[case] expected_path: &str,
  ) {
    let mut providers = vec![provider("openai")];
    let mut transport = None;
    let mut models = vec![Model::new("gpt-4o", "openai")];
    let mut defaults = Defaults::default();
    let mut agents = Vec::new();

    match case_id {
      "Providers are empty.Invalid" => providers.clear(),
      "Models are empty.Invalid" => models.clear(),
      "Transport timeout is zero.Invalid" => {
        transport = Some(
          TransportBuilder::default()
            .timeout_ms(0)
            .build()
            .expect("no required fields"),
        );
      }
      "Temperature is out of range.Invalid" => {
        models = vec![model_with(
          GenerationBuilder::default()
            .temperature(2.5)
            .build()
            .expect("no required fields"),
        )];
      }
      "Top p is out of range.Invalid" => {
        models = vec![model_with(
          GenerationBuilder::default()
            .top_p(1.5)
            .build()
            .expect("no required fields"),
        )];
      }
      "Retries exceed the maximum.Invalid" => {
        providers = vec![
          ProviderBuilder::default()
            .name("openai".to_owned())
            .kind(ProviderId::resolve("openai").expect("registered"))
            .api_key(Secret::Literal("test-api-key".to_owned()))
            .retries(
              RetriesBuilder::default()
                .max_retries(11)
                .backoff_ms(10)
                .build()
                .expect("no required fields"),
            )
            .build()
            .expect("provider builder has all required fields"),
        ];
      }
      "Provider header is malformed.Invalid" => {
        providers = vec![
          ProviderBuilder::default()
            .name("openai".to_owned())
            .kind(ProviderId::resolve("openai").expect("registered"))
            .api_key(Secret::Literal("test-api-key".to_owned()))
            .headers(std::collections::HashMap::from([(
              "bad header".to_owned(),
              "value".to_owned(),
            )]))
            .build()
            .expect("provider builder has all required fields"),
        ];
      }
      "Provider credential is missing.Invalid" => {
        providers = vec![
          ProviderBuilder::default()
            .name("openai".to_owned())
            .kind(ProviderId::resolve("openai").expect("registered"))
            .build()
            .expect("provider builder has all required fields"),
        ];
      }
      "Identity with a non-OpenAI provider.Invalid" => {
        providers.push(provider_of("backup", "anthropic"));
        transport = Some(
          TransportBuilder::default()
            .identity(CallerIdentity::new("carisa"))
            .build()
            .expect("no required fields"),
        );
      }
      "Model provider is unknown.Invalid" => {
        *models.first_mut().expect("model exists for this case") =
          Model::new("gpt-4o", "missing");
      }
      "Default model is unknown.Invalid" => {
        defaults = Defaults::default().with_model("missing");
      }
      "Agent model is unknown.Invalid" => {
        agents.push(
          crate::agent::AgentBuilder::default()
            .instructions("Instructions".to_owned())
            .model("missing")
            .build()
            .expect("instructions provided"),
        );
      }
      _ => unreachable!("all validation cases are listed"),
    }

    let platform = AgentPlatform::new(
      providers,
      transport,
      models,
      defaults,
      agents,
      Arc::new(InMemoryLogStorage::new()),
    );
    let error = platform.validate().expect_err("case is invalid");

    assert!(error.to_string().contains(expected_path));
  }

  /// A `gpt-4o` model definition with `generation`.
  fn model_with(generation: Generation) -> Model {
    ModelBuilder::default()
      .id("gpt-4o".to_owned())
      .provider("openai".to_owned())
      .generation(generation)
      .build()
      .expect("model builder has all required fields")
  }

  #[rstest]
  #[case(
    "File loads all fields.All values preserved",
    r#"{
      "schema_version": 1,
      "providers": {"openai": {
        "name": "openai",
        "kind": "openai",
        "api_key": {"literal": "file-key"},
        "base_url": "https://provider.example",
        "headers": {"x-test": "yes"},
        "retries": {"max_retries": 7, "backoff_ms": 125},
        "tls": {"insecure_skip_verify": true, "ca_file": "/tmp/ca.pem"}
      }},
      "transport": {
        "timeout_ms": 30000,
        "connect_timeout_ms": 5000,
        "proxy": "http://proxy.example:3128",
        "default_headers": {"x-platform": "yes"}
      },
      "models": {"gpt-4o": {
        "id": "gpt-4o",
        "name": "GPT 4o",
        "provider": "openai",
        "generation": {
          "temperature": 0.25,
          "max_tokens": 2048,
          "top_p": 0.9,
          "seed": 7,
          "additional_params": {"reasoning_effort": "high"}
        }
      }},
      "defaults": {"model": "gpt-4o"}
    }"#
  )]
  #[case(
    "File omits optional fields.Defaults applied",
    r#"{
      "schema_version": 1,
      "providers": {"openai": {
        "name": "openai",
        "kind": "openai",
        "api_key": {"literal": "file-key"}
      }},
      "models": {"gpt-4o": {
        "id": "gpt-4o",
        "provider": "openai"
      }}
    }"#
  )]
  fn from_file_deserializes_complete_and_optional_configs(
    #[case] case_id: &str,
    #[case] content: &str,
  ) {
    let builder = load_file_config(content);
    let platform = builder
      .log_storage(Arc::new(InMemoryLogStorage::new()))
      .build()
      .expect("file config builds");
    let loaded_provider = platform.providers.first().expect("provider loaded");
    let loaded_model = platform.models.first().expect("model loaded");

    assert_eq!(platform.providers.len(), 1);
    assert_eq!(platform.models.len(), 1);
    assert!(platform.agents.is_empty(), "agents are builder-only");
    assert_serde_round_trip(loaded_provider);
    assert_serde_round_trip(&platform.transport);
    assert_serde_round_trip(loaded_model);
    assert_serde_round_trip(&platform.defaults);

    if case_id.starts_with("File loads all fields") {
      assert_eq!(
        loaded_provider.kind(),
        &ProviderId::resolve("openai").expect("openai is registered")
      );
      assert_eq!(
        loaded_provider.api_key(),
        Some(&Secret::Literal("file-key".into()))
      );
      assert_eq!(
        loaded_provider.base_url().map(url::Url::as_str),
        Some("https://provider.example/")
      );
      assert_eq!(
        loaded_provider.headers().get("x-test").map(String::as_str),
        Some("yes")
      );
      assert_eq!(loaded_provider.retries().max_retries(), 7);
      assert_eq!(loaded_provider.retries().backoff_ms(), 125);
      assert_eq!(
        loaded_provider.tls(),
        Some(
          &TlsBuilder::default()
            .insecure_skip_verify(true)
            .ca_file("/tmp/ca.pem")
            .build()
            .expect("TLS builder has no required fields")
        )
      );
      let transport = platform.transport.as_ref().expect("transport loaded");
      assert_eq!(transport.timeout_ms(), Some(30_000));
      assert_eq!(transport.connect_timeout_ms(), Some(5_000));
      assert_eq!(
        transport.proxy().map(url::Url::as_str),
        Some("http://proxy.example:3128/")
      );
      assert_eq!(
        transport
          .default_headers()
          .get("x-platform")
          .map(String::as_str),
        Some("yes")
      );
      assert_eq!(loaded_model.name().map(String::as_str), Some("GPT 4o"));
      assert_eq!(loaded_model.generation().temperature(), Some(0.25));
      assert_eq!(loaded_model.generation().max_tokens(), Some(2048));
      assert_eq!(loaded_model.generation().top_p(), Some(0.9));
      assert_eq!(loaded_model.generation().seed(), Some(7));
      assert_eq!(
        loaded_model
          .generation()
          .additional_params()
          .get("reasoning_effort"),
        Some(&serde_json::json!("high"))
      );
      assert_eq!(
        platform.defaults.model().map(String::as_str),
        Some("gpt-4o")
      );
    } else {
      assert_eq!(loaded_provider.retries(), &Retries::default());
      assert!(loaded_provider.base_url().is_none());
      assert!(loaded_provider.headers().is_empty());
      assert!(loaded_provider.tls().is_none());
      assert!(platform.transport.is_none());
      assert!(loaded_model.name().is_none());
      assert_eq!(
        loaded_model.generation(),
        &crate::agent::config::Generation::default()
      );
      assert!(platform.defaults.model().is_none());
    }
  }

  #[test]
  fn builder_values_upsert_file_providers_and_models() {
    let builder = load_file_config(
      r#"{
        "schema_version": 1,
        "providers": {"original": {
          "name": "original",
          "kind": "openai",
          "api_key": {"literal": "file-key"}
        }},
        "models": {"original-model": {
          "id": "original-model",
          "provider": "original"
        }},
        "defaults": {"model": "original-model"}
      }"#,
    );
    let agent = crate::agent::AgentBuilder::default()
      .instructions("Builder agent".to_owned())
      .build()
      .expect("instructions provided");
    let updated_provider = ProviderBuilder::default()
      .name("original".to_owned())
      .kind(ProviderId::resolve("openai").expect("registered"))
      .api_key(Secret::Literal("builder-key".to_owned()))
      .build()
      .expect("provider builder has all required fields");
    let updated_model = ModelBuilder::default()
      .id("original-model".to_owned())
      .provider("original".to_owned())
      .name("Builder model")
      .build()
      .expect("model builder has all required fields");
    let platform = builder
      .providers(vec![provider("replacement"), updated_provider])
      .models(vec![
        Model::new("replacement-model", "replacement"),
        updated_model,
      ])
      .default_model("replacement-model")
      .agent(agent)
      .log_storage(Arc::new(InMemoryLogStorage::new()))
      .build()
      .expect("builder overrides file values");

    assert_eq!(platform.providers.len(), 2);
    assert_eq!(
      platform
        .providers
        .iter()
        .find(|item| item.name() == "replacement")
        .map(Provider::name),
      Some("replacement")
    );
    assert_eq!(
      platform
        .providers
        .iter()
        .find(|item| item.name() == "original")
        .and_then(Provider::api_key),
      Some(&Secret::Literal("builder-key".to_owned()))
    );
    assert_eq!(platform.models.len(), 2);
    assert_eq!(
      platform
        .models
        .iter()
        .find(|item| item.id() == "replacement-model")
        .map(Model::id),
      Some("replacement-model")
    );
    assert_eq!(
      platform
        .models
        .iter()
        .find(|item| item.id() == "original-model")
        .and_then(Model::name)
        .map(String::as_str),
      Some("Builder model")
    );
    assert_eq!(
      platform.defaults.model().map(String::as_str),
      Some("replacement-model")
    );
    assert_eq!(platform.agents.len(), 1);
    assert_eq!(
      platform
        .agents
        .first()
        .map(crate::agent::Agent::instructions),
      Some("Builder agent")
    );
  }

  #[test]
  fn repeated_json_sources_upsert_providers_and_models_by_id() {
    let mut builder = AgentPlatformBuilder::default();
    builder
      .merge_json_value(serde_json::json!({
        "schema_version": 1,
        "providers": {"openai": {
          "name": "openai",
          "kind": "openai",
          "api_key": {"literal": "first-key"}
        }},
        "models": {"model-id": {
          "id": "model-id",
          "provider": "openai"
        }}
      }))
      .expect("first source merges");
    builder
      .merge_json_value(serde_json::json!({
        "schema_version": 1,
        "providers": {"openai": {
          "name": "openai",
          "kind": "openai",
          "api_key": {"literal": "second-key"}
        }},
        "models": {"model-id": {
          "id": "model-id",
          "name": "Replacement",
          "provider": "openai"
        }}
      }))
      .expect("second source merges");

    assert_eq!(builder.providers.as_ref().map(Vec::len), Some(1));
    assert_eq!(
      builder
        .providers
        .as_ref()
        .and_then(|items| items.first())
        .and_then(Provider::api_key),
      Some(&Secret::Literal("second-key".to_owned()))
    );
    assert_eq!(builder.models.as_ref().map(Vec::len), Some(1));
    assert_eq!(
      builder
        .models
        .as_ref()
        .and_then(|items| items.first())
        .and_then(Model::name)
        .map(String::as_str),
      Some("Replacement")
    );
  }

  #[rstest]
  #[case(
    "Provider map key differs from name",
    r#"{"providers": {"alias": {"name": "openai", "kind": "openai",
        "api_key": {"literal": "key"}}}}"#,
    "providers.alias"
  )]
  #[case(
    "Model map key differs from id",
    r#"{"models": {"alias": {"id": "gpt-4o", "provider": "openai"}}}"#,
    "models.alias"
  )]
  #[case("Unknown section", r#"{"providers_typo": {}}"#, "providers_typo")]
  #[case(
    "Unknown provider kind",
    r#"{"providers": {"openai": {"name": "openai", "kind": "nope"}}}"#,
    "providers.openai.kind"
  )]
  #[case(
    "Ambiguous provider kind",
    r#"{"providers": {"minimax": {"name": "minimax", "kind": "minimax"}}}"#,
    "providers.minimax.kind"
  )]
  #[case(
    "Removed openai_compatible kind",
    r#"{"providers": {"gateway": {"name": "gateway",
        "kind": "openai_compatible"}}}"#,
    "providers.gateway.kind"
  )]
  fn json_sources_reject_unknown_keys_and_kinds(
    #[case] case_id: &str,
    #[case] section: &str,
    #[case] expected_path: &str,
  ) {
    assert_ne!(case_id, "");
    let mut root: serde_json::Value =
      serde_json::from_str(section).expect("case is valid JSON");
    root
      .as_object_mut()
      .expect("every case is an object")
      .insert("schema_version".to_owned(), serde_json::json!(1));
    let error = AgentPlatformBuilder::default()
      .merge_json_value(root)
      .expect_err("the source is invalid");

    assert!(
      error.to_string().contains(expected_path),
      "`{case_id}` reported `{error}`, expected `{expected_path}`"
    );
  }

  #[test]
  fn provider_kind_hints_at_the_openai_gateway_composition() {
    let error = AgentPlatformBuilder::default()
      .merge_json_value(serde_json::json!({
        "schema_version": 1,
        "providers": {"gateway": {
          "name": "gateway",
          "kind": "openai_compatible"
        }}
      }))
      .expect_err("the kind was removed");

    assert!(error.to_string().contains("base_url"));
  }

  #[test]
  fn ambiguous_kind_names_the_qualified_alternatives() {
    let error = AgentPlatformBuilder::default()
      .merge_json_value(serde_json::json!({
        "schema_version": 1,
        "providers": {"minimax": {"name": "minimax", "kind": "minimax"}}
      }))
      .expect_err("minimax registers more than one format");

    assert!(error.to_string().contains("minimax/anthropic"));
    assert!(error.to_string().contains("minimax/openai"));
  }

  #[rstest]
  #[case("Missing schema version", "{}")]
  #[case("Unsupported schema version", r#"{"schema_version": 2}"#)]
  #[case("Non-numeric schema version", r#"{"schema_version": "1"}"#)]
  fn schema_version_is_required_and_pinned(
    #[case] case_id: &str,
    #[case] content: &str,
  ) {
    assert_ne!(case_id, "");
    let value: serde_json::Value =
      serde_json::from_str(content).expect("case is valid JSON");

    assert!(
      AgentPlatformBuilder::default()
        .merge_json_value(value)
        .is_err()
    );
  }

  #[test]
  fn from_env_path_loads_the_named_file() {
    let path = std::env::temp_dir()
      .join(format!("carisa-platform-{}.json", ulid::Ulid::generate()));
    fs::write(
      &path,
      r#"{
        "schema_version": 1,
        "providers": {"openai": {
          "name": "openai",
          "kind": "openai",
          "api_key": {"env": "OPENAI_API_KEY"}
        }},
        "models": {"gpt-4o": {"id": "gpt-4o", "provider": "openai"}},
        "defaults": {"model": "gpt-4o"}
      }"#,
    )
    .expect("temporary config is written");

    let output = run_child(Some(path.as_os_str()), "ok");
    let removed = fs::remove_file(&path);

    assert!(
      output.status.success(),
      "child test failed: {}{}",
      String::from_utf8_lossy(&output.stdout),
      String::from_utf8_lossy(&output.stderr)
    );
    removed.expect("temporary config is removed");
  }

  #[test]
  fn from_env_path_requires_the_variable() {
    let output = run_child(None, "missing");

    assert!(
      output.status.success(),
      "child test failed: {}{}",
      String::from_utf8_lossy(&output.stdout),
      String::from_utf8_lossy(&output.stderr)
    );
  }

  /// Runs [`Self::from_env_path_child`] in a fresh process, so the
  /// environment can differ from the test runner's.
  fn run_child(
    path: Option<&std::ffi::OsStr>,
    expectation: &str,
  ) -> std::process::Output {
    let test_binary =
      std::env::current_exe().expect("test binary is available");
    let mut command = std::process::Command::new(test_binary);
    command
      .args([
        "--exact",
        "agent::platform::tests::from_env_path_child",
        "--nocapture",
      ])
      .env("RUST_TEST_THREADS", "1")
      .env("CARISA_TEST_CHILD", "1")
      .env("CARISA_TEST_EXPECT", expectation)
      .env_remove("CARISA_CONFIG_PATH");
    if let Some(path) = path {
      command.env("CARISA_CONFIG_PATH", path);
    }

    command.output().expect("child test process runs")
  }

  #[test]
  fn from_env_path_child() {
    if std::env::var_os("CARISA_TEST_CHILD").is_none() {
      return;
    }

    let expectation = std::env::var("CARISA_TEST_EXPECT")
      .expect("parent provides an expectation");
    match expectation.as_str() {
      "ok" => {
        let platform = AgentPlatformBuilder::from_env_path()
          .expect("the variable names a readable file")
          .log_storage(Arc::new(InMemoryLogStorage::new()))
          .build()
          .expect("the file config builds");
        assert_eq!(platform.providers.len(), 1);
        assert_eq!(
          platform.defaults.model().map(String::as_str),
          Some("gpt-4o")
        );
      }
      "missing" => {
        let error = AgentPlatformBuilder::from_env_path()
          .expect_err("the variable is absent");
        assert!(error.to_string().contains("CARISA_CONFIG_PATH"));
      }
      other => unreachable!("unknown expectation `{other}`"),
    }
  }

  #[test]
  fn platform_requires_at_least_one_provider() {
    let error = AgentPlatformBuilder::default()
      .providers(vec![])
      .models(vec![Model::new("gpt-4o", "openai")])
      .log_storage(Arc::new(InMemoryLogStorage::new()))
      .build()
      .expect_err("an empty provider vector is invalid");

    assert!(error.to_string().contains("providers"));
  }

  #[test]
  fn platform_requires_at_least_one_model() {
    let error = AgentPlatformBuilder::default()
      .providers(vec![provider("openai"), provider("backup")])
      .models(vec![])
      .log_storage(Arc::new(InMemoryLogStorage::new()))
      .build()
      .expect_err("an empty model vector is invalid");

    assert!(error.to_string().contains("models"));
  }

  #[test]
  fn provider_without_a_base_url_uses_the_dialect_endpoint() {
    let platform = valid_builder()
      .build()
      .expect("a provider inherits the dialect's endpoint");

    assert!(platform.providers().iter().all(|p| p.base_url().is_none()));
  }

  #[test]
  fn transport_is_shared_and_optional() {
    let transport = TransportBuilder::default()
      .timeout_ms(1200)
      .build()
      .expect("no required fields");
    let platform = valid_builder()
      .transport(transport.clone())
      .build()
      .expect("a shared transport is enough");

    assert_eq!(platform.transport(), Some(&transport));

    let without = valid_builder().build().expect("transport is optional");

    assert!(without.transport().is_none());
  }

  #[test]
  fn invalid_base_url_is_rejected_when_loading_json() {
    let mut builder = AgentPlatformBuilder::default();
    let error = builder
      .merge_json_value(serde_json::json!({
        "schema_version": 1,
        "providers": {"gateway": {
          "name": "gateway",
          "kind": "openai",
          "base_url": "not a url"
        }}
      }))
      .expect_err("the URL is malformed");

    assert!(error.to_string().contains("providers.gateway"));
  }

  #[rstest]
  #[case("OpenAI requires a credential", "openai", true)]
  #[case("Ollama takes an optional credential", "ollama/openai", false)]
  fn provider_credential_is_required_only_when_the_kind_needs_it(
    #[case] case_id: &str,
    #[case] kind: &str,
    #[case] rejects: bool,
  ) {
    assert_ne!(case_id, "");
    let provider = ProviderBuilder::default()
      .name("local".to_owned())
      .kind(ProviderId::resolve(kind).expect("kind is registered"))
      .build()
      .expect("provider builder has all required fields");
    let platform = AgentPlatform::new(
      vec![provider],
      None,
      vec![Model::new("gpt-4o", "local")],
      Defaults::default(),
      Vec::new(),
      Arc::new(InMemoryLogStorage::new()),
    );

    assert_eq!(platform.validate().is_err(), rejects);
  }

  #[test]
  fn catalog_rejects_temperature_a_model_cannot_take() {
    let error = AgentPlatformBuilder::default()
      .providers(vec![
        ProviderBuilder::default()
          .name("anthropic".to_owned())
          .kind(ProviderId::resolve("anthropic").expect("registered"))
          .api_key(Secret::Literal("test-api-key".to_owned()))
          .build()
          .expect("provider builder has all required fields"),
      ])
      .models(vec![
        ModelBuilder::default()
          .id("claude-fable-5".to_owned())
          .provider("anthropic".to_owned())
          .generation(
            GenerationBuilder::default()
              .temperature(0.5)
              .build()
              .expect("no required fields"),
          )
          .build()
          .expect("model builder has all required fields"),
      ])
      .default_model("claude-fable-5")
      .log_storage(Arc::new(InMemoryLogStorage::new()))
      .build()
      .expect_err("the model refuses a temperature");

    assert!(
      error
        .to_string()
        .contains("models.claude-fable-5.temperature"),
      "unexpected error: {error}"
    );
  }

  #[test]
  fn defaults_block_is_optional() {
    let platform = valid_builder().build().expect("defaults may be omitted");

    assert!(platform.defaults.model().is_none());
  }

  #[test]
  fn invalid_provider_json_is_not_silently_discarded() {
    let mut builder = AgentPlatformBuilder::default();
    let result = builder.merge_json_value(serde_json::json!({
      "schema_version": 1,
      "providers": {
        "openai": {
          "name": "openai"
        }
      }
    }));

    assert!(
      result
        .expect_err("a provider without `kind` must fail deserialization")
        .to_string()
        .contains("providers.openai")
    );
  }

  #[test]
  fn log_storage_is_required_to_build() {
    let error = AgentPlatformBuilder::default()
      .providers(vec![provider("openai"), provider("backup")])
      .models(vec![Model::new("gpt-4o", "openai")])
      .build()
      .expect_err("log storage is required");

    assert!(error.to_string().contains("log_storage"));
  }
}
