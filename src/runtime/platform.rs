//! Builder and validated runtime platform.
//!
//! The builder accepts provider, HTTP, model, and default configuration from
//! JSON or `CARISA_` environment variables. Both loaders start with an empty
//! builder and return the same builder type; they do not load one another.
//! Agents and log storage are configured in Rust. See the parent
//! [`runtime`](super) module for full JSON and environment examples.

use std::{fs, path::Path, sync::Arc};

use derive_builder::Builder;

use crate::memory::session::LogStorage;

use super::{
  config::{Defaults, Http, Model, Provider},
  error::PlatformError,
};

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
  /// Optional platform-wide HTTP settings.
  #[builder(default, setter(strip_option))]
  http: Option<Http>,
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
      .field("http", &self.http)
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
      .field("http", &self.http)
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

  /// Loads provider, HTTP, model, and default settings from a JSON file.
  ///
  /// The root JSON object may contain `providers`, `http`, `models`, and
  /// `defaults`. Provider and model collections must be objects; their
  /// entries are deserialized as complete values. A JSON `agents` property
  /// is ignored. This creates a fresh builder; use its methods to add agents
  /// or override loaded values.
  ///
  /// Providers are keyed for replacement by their `name`, and models by
  /// their `id`. Replacing an existing key replaces the complete value, not
  /// just the fields present in a later source.
  ///
  /// # Errors
  ///
  /// Returns an error when the file cannot be opened or parsed.
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

  /// Loads provider, HTTP, model, and default settings from `CARISA_`
  /// environment variables.
  ///
  /// Double underscores (`__`) separate nested path components, and each
  /// component is converted to lowercase. For example,
  /// `CARISA_PROVIDERS__OPENAI__API_KEY__ENV=OPENAI_API_KEY` sets the
  /// provider's secret to the `Env` variant. Values that are valid JSON are
  /// parsed as JSON values, so numbers, booleans, arrays, and objects retain
  /// their types; other values remain strings. `CARISA_AGENTS` is ignored:
  /// agents are supplied through the builder, not the environment.
  ///
  /// This creates a fresh builder rather than layering environment values on
  /// a file-loaded builder. Use the builder methods to add or replace values
  /// after loading. Existing providers are replaced by `name` and models by
  /// `id`; each replacement is complete, not a recursive field merge.
  ///
  /// # Errors
  ///
  /// Returns an error if a nested value cannot be merged into the
  /// configuration tree.
  pub fn from_env() -> Result<Self, PlatformError> {
    let mut root = serde_json::Value::Object(serde_json::Map::new());

    for (key, value) in std::env::vars() {
      if let Some(stripped) = key.strip_prefix("CARISA_") {
        let path = stripped
          .split("__")
          .map(str::to_ascii_lowercase)
          .collect::<Vec<_>>();
        insert_nested_value(&mut root, &path, parse_env_value(value));
      }
    }

    let mut builder = Self::default();
    builder.merge_json_value(root)?;
    Ok(builder)
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
    if let serde_json::Value::Object(map) = value {
      for (key, val) in map {
        match key.as_str() {
          "providers" => {
            if let serde_json::Value::Object(entries) = val {
              for (name, provider_value) in entries {
                let provider = serde_json::from_value::<Provider>(
                  provider_value,
                )
                .map_err(|source| PlatformError::Json {
                  path: format!("providers.{name}"),
                  source,
                })?;
                upsert_by(
                  self.providers.get_or_insert_with(Vec::new),
                  provider,
                  |item| item.name(),
                );
              }
            }
          }
          "http" => {
            self.http =
              Some(Some(serde_json::from_value::<Http>(val).map_err(
                |source| PlatformError::Json {
                  path: "http".to_owned(),
                  source,
                },
              )?));
          }
          "models" => {
            if let serde_json::Value::Object(entries) = val {
              for (name, model_value) in entries {
                let model = serde_json::from_value::<Model>(model_value)
                  .map_err(|source| PlatformError::Json {
                    path: format!("models.{name}"),
                    source,
                  })?;
                upsert_by(
                  self.models.get_or_insert_with(Vec::new),
                  model,
                  |item| item.id(),
                );
              }
            }
          }
          "defaults" => {
            self.defaults =
              Some(serde_json::from_value::<Defaults>(val).map_err(
                |source| PlatformError::Json {
                  path: "defaults".to_owned(),
                  source,
                },
              )?);
          }
          _ => {}
        }
      }
    }

    Ok(())
  }
}

impl AgentPlatform {
  /// Creates a platform from resolved configuration values.
  pub fn new(
    providers: Vec<Provider>,
    http: Option<Http>,
    models: Vec<Model>,
    defaults: Defaults,
    agents: Vec<crate::agent::Agent>,
    log_storage: Arc<dyn LogStorage>,
  ) -> Self {
    Self {
      providers,
      http,
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

  /// Returns the optional platform-wide HTTP settings.
  pub const fn http(&self) -> Option<&Http> {
    self.http.as_ref()
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

  /// Validates internal references for the platform.
  ///
  /// # Errors
  ///
  /// Returns an error when providers or models are missing, HTTP settings are
  /// unavailable, or a reference points to a missing target.
  pub fn validate(&self) -> Result<(), PlatformError> {
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

    if let Some(http) = &self.http {
      validate_http("http", http)?;
    }

    for provider in &self.providers {
      if let Some(http) = provider.http() {
        validate_http(&format!("providers.{}.http", provider.name()), http)?;
      }
      if self.http.is_none() && provider.http().is_none() {
        return Err(PlatformError::invalid(
          format!("providers.{}.http", provider.name()),
          "provider HTTP settings are required when platform HTTP is absent",
        ));
      }
    }

    for model in &self.models {
      if !self
        .providers
        .iter()
        .any(|provider| provider.name() == model.provider())
      {
        return Err(PlatformError::invalid(
          format!("models.{}.provider", model.id()),
          "referenced provider does not exist",
        ));
      }
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
      if let Some(m) = agent.model()
        && !self.models.iter().any(|model| model.id() == m.id())
      {
        return Err(PlatformError::invalid(
          format!("agents[{idx}].model.id"),
          "referenced model does not exist",
        ));
      }
    }

    Ok(())
  }
}

fn validate_http(path: &str, http: &Http) -> Result<(), PlatformError> {
  if http.base_url().trim().is_empty() {
    return Err(PlatformError::invalid(
      format!("{path}.base_url"),
      "base URL must not be empty",
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

fn insert_nested_value(
  target: &mut serde_json::Value,
  path: &[String],
  value: serde_json::Value,
) {
  let mut current = target;
  for part in path.iter().take(path.len().saturating_sub(1)) {
    if !current.is_object() {
      *current = serde_json::Value::Object(serde_json::Map::new());
    }
    let Some(obj) = current.as_object_mut() else {
      return;
    };
    current = obj
      .entry(part.clone())
      .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
  }

  if let Some(last) = path.last() {
    if !current.is_object() {
      *current = serde_json::Value::Object(serde_json::Map::new());
    }
    let Some(object) = current.as_object_mut() else {
      return;
    };
    object.insert(last.clone(), value);
  }
}

fn parse_env_value(value: String) -> serde_json::Value {
  serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{
    memory::session::inmemory::InMemoryLogStorage,
    runtime::config::{
      Generation, ModelBuilder, ProviderBuilder, Retries, TlsBuilder,
    },
    runtime::{ProviderKind, Secret},
  };
  use rstest::rstest;

  fn provider(name: &str) -> Provider {
    provider_with_http(name, Http::new(format!("https://{name}.example")))
  }

  fn provider_with_http(name: &str, http: Http) -> Provider {
    ProviderBuilder::default()
      .name(name.to_owned())
      .kind(ProviderKind::OpenAI)
      .api_key(Secret::Literal("test-api-key".to_owned()))
      .http(http)
      .build()
      .expect("provider builder has all required fields")
  }

  fn provider_without_http(name: &str) -> Provider {
    ProviderBuilder::default()
      .name(name.to_owned())
      .kind(ProviderKind::OpenAI)
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
  #[case("Platform HTTP URL is blank.Invalid", "http.base_url")]
  #[case(
    "Provider HTTP URL is blank.Invalid",
    "providers.openai.http.base_url"
  )]
  #[case("Provider HTTP is absent.Invalid", "providers.openai.http")]
  #[case("Model provider is unknown.Invalid", "models.gpt-4o.provider")]
  #[case("Default model is unknown.Invalid", "defaults.model")]
  #[case("Agent model is unknown.Invalid", "agents[0].model.id")]
  fn validate_rejects_each_invalid_platform_case(
    #[case] case_id: &str,
    #[case] expected_path: &str,
  ) {
    let mut providers = vec![provider("openai")];
    let mut http = None;
    let mut models = vec![Model::new("gpt-4o", "openai")];
    let mut defaults = Defaults::default();
    let mut agents = Vec::new();

    match case_id {
      "Providers are empty.Invalid" => providers.clear(),
      "Models are empty.Invalid" => models.clear(),
      "Platform HTTP URL is blank.Invalid" => {
        http = Some(Http::new(" "));
      }
      "Provider HTTP URL is blank.Invalid" => {
        providers = vec![provider_with_http("openai", Http::new(" "))];
      }
      "Provider HTTP is absent.Invalid" => {
        providers = vec![provider_without_http("openai")];
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
      http,
      models,
      defaults,
      agents,
      Arc::new(InMemoryLogStorage::new()),
    );
    let error = platform.validate().expect_err("case is invalid");

    assert!(error.to_string().contains(expected_path));
  }

  #[rstest]
  #[case(
    "File loads all fields.All values preserved",
    r#"{
      "providers": {"openai": {
        "name": "openai",
        "kind": "openai",
        "api_key": {"literal": "file-key"},
        "http": {
          "base_url": "https://provider.example",
          "headers": {"x-test": "yes"},
          "options": {"timeout": 30}
        },
        "retries": {"max_retries": 7, "backoff_ms": 125},
        "tls": {"insecure_skip_verify": true, "ca_file": "/tmp/ca.pem"}
      }},
      "http": {
        "base_url": "https://platform.example",
        "headers": {"x-platform": "yes"},
        "options": {"proxy": "https://proxy.example"}
      },
      "models": {"gpt-4o": {
        "id": "gpt-4o",
        "name": "GPT 4o",
        "provider": "openai",
        "generation": {
          "temperature": 0.25,
          "max_tokens": 2048,
          "additional_params": {"reasoning_effort": "high"}
        }
      }},
      "defaults": {"model": "gpt-4o"},
      "agents": [{"instructions": "External agent"}]
    }"#
  )]
  #[case(
    "File omits optional fields.Defaults applied",
    r#"{
      "providers": {"openai": {
        "name": "openai",
        "kind": "openai",
        "api_key": {"literal": "file-key"}
      }},
      "http": {"base_url": "https://platform.example"},
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
    assert_serde_round_trip(&platform.http);
    assert_serde_round_trip(loaded_model);
    assert_serde_round_trip(&platform.defaults);

    if case_id.starts_with("File loads all fields") {
      assert_eq!(loaded_provider.kind(), &ProviderKind::OpenAI);
      assert_eq!(
        loaded_provider.api_key(),
        &Secret::Literal("file-key".into())
      );
      let provider_http = loaded_provider.http().expect("provider HTTP loaded");
      assert_eq!(provider_http.base_url(), "https://provider.example");
      assert_eq!(
        provider_http.headers().get("x-test").map(String::as_str),
        Some("yes")
      );
      assert_eq!(
        provider_http.options().get("timeout"),
        Some(&serde_json::json!(30))
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
      let platform_http = platform.http.as_ref().expect("platform HTTP loaded");
      assert_eq!(platform_http.base_url(), "https://platform.example");
      assert_eq!(
        platform_http
          .headers()
          .get("x-platform")
          .map(String::as_str),
        Some("yes")
      );
      assert_eq!(
        platform_http.options().get("proxy"),
        Some(&serde_json::json!("https://proxy.example"))
      );
      assert_eq!(loaded_model.name().map(String::as_str), Some("GPT 4o"));
      assert_eq!(loaded_model.generation().temperature(), Some(0.25));
      assert_eq!(loaded_model.generation().max_tokens(), Some(2048));
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
      assert!(loaded_provider.http().is_none());
      assert!(loaded_provider.tls().is_none());
      assert!(loaded_model.name().is_none());
      assert_eq!(loaded_model.generation(), &Generation::default());
      assert!(platform.defaults.model().is_none());
    }
  }

  #[test]
  fn builder_values_upsert_file_providers_and_models() {
    let builder = load_file_config(
      r#"{
        "providers": {"original": {
          "name": "original",
          "kind": "openai",
          "api_key": {"literal": "file-key"},
          "http": {"base_url": "https://original.example"}
        }},
        "models": {"original-model": {
          "id": "original-model",
          "provider": "original"
        }},
        "defaults": {"model": "original-model"},
        "agents": [{"instructions": "Ignored"}]
      }"#,
    );
    let agent = crate::agent::AgentBuilder::default()
      .instructions("Builder agent".to_owned())
      .build()
      .expect("instructions provided");
    let updated_provider = ProviderBuilder::default()
      .name("original".to_owned())
      .kind(ProviderKind::OpenAI)
      .api_key(Secret::Literal("builder-key".to_owned()))
      .http(Http::new("https://original.example"))
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
        .map(Provider::api_key),
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
        "providers": {"first": {
          "name": "openai",
          "kind": "openai",
          "api_key": {"literal": "first-key"},
          "http": {"base_url": "https://first.example"}
        }},
        "models": {"first": {
          "id": "model-id",
          "provider": "openai"
        }}
      }))
      .expect("first source merges");
    builder
      .merge_json_value(serde_json::json!({
        "providers": {"second": {
          "name": "openai",
          "kind": "openai",
          "api_key": {"literal": "second-key"},
          "http": {"base_url": "https://second.example"}
        }},
        "models": {"second": {
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
        .map(Provider::api_key),
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
  #[case("Environment loads all fields.All values preserved", "full")]
  #[case("Environment omits optional fields.Defaults applied", "minimal")]
  fn from_env_loads_in_isolated_process(
    #[case] case_id: &str,
    #[case] scenario: &str,
  ) {
    assert!(case_id.starts_with("Environment "));
    let test_binary =
      std::env::current_exe().expect("test binary is available");
    let inherited = std::env::vars_os()
      .filter(|(key, _)| !key.to_string_lossy().starts_with("CARISA_"));
    let mut command = std::process::Command::new(test_binary);
    command
      .args([
        "--exact",
        "runtime::platform::tests::from_env_child_process",
        "--nocapture",
      ])
      .env_clear()
      .envs(inherited)
      .env("RUST_TEST_THREADS", "1")
      .env("CARISA_TEST_CHILD", "1")
      .env("CARISA_TEST_CASE", scenario);

    for (key, value) in env_config(scenario) {
      command.env(key, value);
    }

    let output = command.output().expect("child test process runs");
    assert!(
      output.status.success(),
      "child test failed: {}{}",
      String::from_utf8_lossy(&output.stdout),
      String::from_utf8_lossy(&output.stderr)
    );
  }

  fn env_config(scenario: &str) -> Vec<(&'static str, &'static str)> {
    let common = vec![
      ("CARISA_PROVIDERS__OPENAI__NAME", "openai"),
      ("CARISA_PROVIDERS__OPENAI__KIND", "openai"),
      ("CARISA_PROVIDERS__OPENAI__API_KEY__LITERAL", "env-key"),
      ("CARISA_HTTP__BASE_URL", "https://platform.example"),
      ("CARISA_MODELS__GPT-4O__ID", "gpt-4o"),
      ("CARISA_MODELS__GPT-4O__PROVIDER", "openai"),
      ("CARISA_AGENTS", r#"[{"instructions":"External agent"}]"#),
    ];
    if scenario == "minimal" {
      return common;
    }

    [
      common,
      vec![
        (
          "CARISA_PROVIDERS__OPENAI__HTTP__BASE_URL",
          "https://provider.example",
        ),
        ("CARISA_PROVIDERS__OPENAI__HTTP__HEADERS__X-TEST", "yes"),
        ("CARISA_PROVIDERS__OPENAI__HTTP__OPTIONS__TIMEOUT", "30"),
        ("CARISA_PROVIDERS__OPENAI__RETRIES__MAX_RETRIES", "7"),
        ("CARISA_PROVIDERS__OPENAI__RETRIES__BACKOFF_MS", "125"),
        ("CARISA_PROVIDERS__OPENAI__TLS__INSECURE_SKIP_VERIFY", "true"),
        ("CARISA_PROVIDERS__OPENAI__TLS__CA_FILE", "/tmp/ca.pem"),
        ("CARISA_HTTP__HEADERS__X-PLATFORM", "yes"),
        (
          "CARISA_HTTP__OPTIONS__PROXY",
          "https://proxy.example",
        ),
        ("CARISA_MODELS__GPT-4O__NAME", "GPT 4o"),
        (
          "CARISA_MODELS__GPT-4O__GENERATION__TEMPERATURE",
          "0.25",
        ),
        ("CARISA_MODELS__GPT-4O__GENERATION__MAX_TOKENS", "2048"),
        (
          "CARISA_MODELS__GPT-4O__GENERATION__ADDITIONAL_PARAMS__REASONING_EFFORT",
          "high",
        ),
        ("CARISA_DEFAULTS__MODEL", "gpt-4o"),
      ],
    ]
    .concat()
  }

  #[test]
  fn from_env_child_process() {
    if std::env::var_os("CARISA_TEST_CHILD").is_none() {
      return;
    }

    let scenario = std::env::var("CARISA_TEST_CASE")
      .expect("parent provides an environment case");
    let builder = AgentPlatformBuilder::from_env().expect("environment loads");
    let platform = builder
      .clone()
      .log_storage(Arc::new(InMemoryLogStorage::new()))
      .build()
      .expect("environment values build");
    assert_loaded_env_config(&platform, &scenario);

    let agent = crate::agent::AgentBuilder::default()
      .instructions("Builder agent".to_owned())
      .build()
      .expect("instructions provided");
    let overridden = builder
      .providers(vec![provider("replacement"), provider("openai")])
      .models(vec![
        Model::new("replacement-model", "replacement"),
        Model::new("gpt-4o", "replacement"),
      ])
      .default_model("replacement-model")
      .agent(agent)
      .log_storage(Arc::new(InMemoryLogStorage::new()))
      .build()
      .expect("builder overrides environment values");

    assert_eq!(overridden.providers.len(), 2);
    assert_eq!(
      overridden
        .providers
        .iter()
        .find(|item| item.name() == "openai")
        .map(Provider::api_key),
      Some(&Secret::Literal("test-api-key".to_owned()))
    );
    assert_eq!(overridden.models.len(), 2);
    assert_eq!(
      overridden
        .models
        .iter()
        .find(|item| item.id() == "gpt-4o")
        .map(Model::provider),
      Some("replacement")
    );
    assert_eq!(overridden.agents.len(), 1);
  }

  fn assert_loaded_env_config(platform: &AgentPlatform, scenario: &str) {
    let loaded_provider = platform.providers.first().expect("provider loaded");
    let loaded_model = platform.models.first().expect("model loaded");

    assert_eq!(
      loaded_provider.api_key(),
      &Secret::Literal("env-key".into())
    );
    assert_eq!(
      platform.http.as_ref().map(Http::base_url),
      Some("https://platform.example")
    );
    assert!(platform.agents.is_empty(), "agents are builder-only");
    assert_serde_round_trip(loaded_provider);
    assert_serde_round_trip(&platform.http);
    assert_serde_round_trip(loaded_model);
    assert_serde_round_trip(&platform.defaults);

    if scenario == "full" {
      assert_eq!(loaded_provider.kind(), &ProviderKind::OpenAI);
      assert_eq!(loaded_provider.retries().max_retries(), 7);
      assert_eq!(loaded_provider.retries().backoff_ms(), 125);
      let provider_http = loaded_provider.http().expect("provider HTTP loaded");
      assert_eq!(provider_http.base_url(), "https://provider.example");
      assert_eq!(
        provider_http.headers().get("x-test").map(String::as_str),
        Some("yes")
      );
      assert_eq!(
        provider_http.options().get("timeout"),
        Some(&serde_json::json!(30))
      );
      assert_eq!(
        loaded_provider.tls(),
        Some(
          &TlsBuilder::default()
            .insecure_skip_verify(true)
            .ca_file("/tmp/ca.pem".to_owned())
            .build()
            .expect("TLS builder has no required fields")
        )
      );
      let platform_http = platform.http.as_ref().expect("platform HTTP loaded");
      assert_eq!(
        platform_http
          .headers()
          .get("x-platform")
          .map(String::as_str),
        Some("yes")
      );
      assert_eq!(
        platform_http.options().get("proxy"),
        Some(&serde_json::json!("https://proxy.example"))
      );
      assert_eq!(loaded_model.name().map(String::as_str), Some("GPT 4o"));
      assert_eq!(loaded_model.generation().temperature(), Some(0.25));
      assert_eq!(loaded_model.generation().max_tokens(), Some(2048));
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
      assert!(loaded_provider.http().is_none());
      assert!(loaded_provider.tls().is_none());
      assert!(loaded_model.name().is_none());
      assert_eq!(loaded_model.generation(), &Generation::default());
      assert!(platform.defaults.model().is_none());
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
  fn providers_require_http_when_platform_http_is_absent() {
    let provider_without_http = provider_without_http("backup");
    let error = AgentPlatformBuilder::default()
      .providers(vec![provider("openai"), provider_without_http])
      .models(vec![Model::new("gpt-4o", "openai")])
      .log_storage(Arc::new(InMemoryLogStorage::new()))
      .build()
      .expect_err("provider HTTP settings are required");

    assert!(error.to_string().contains("providers.backup.http"));
  }

  #[test]
  fn platform_http_allows_provider_http_to_be_absent() {
    let first = provider_without_http("openai");
    let second = provider_without_http("backup");
    let platform = AgentPlatformBuilder::default()
      .providers(vec![first, second])
      .http(Http::new("https://api.example"))
      .models(vec![Model::new("gpt-4o", "openai")])
      .log_storage(Arc::new(InMemoryLogStorage::new()))
      .build()
      .expect("platform HTTP is sufficient");

    assert!(platform.http.is_some());
  }

  #[test]
  fn platform_rejects_empty_http_base_url() {
    let error = valid_builder()
      .http(Http::new(" "))
      .build()
      .expect_err("a blank base URL is not configured");

    assert!(error.to_string().contains("http.base_url"));
  }

  #[test]
  fn provider_rejects_empty_http_base_url() {
    let provider_with_empty_url = provider_with_http("backup", Http::new(""));
    let error = AgentPlatformBuilder::default()
      .providers(vec![provider("openai"), provider_with_empty_url])
      .models(vec![Model::new("gpt-4o", "openai")])
      .log_storage(Arc::new(InMemoryLogStorage::new()))
      .build()
      .expect_err("a blank base URL is not configured");

    assert!(error.to_string().contains("providers.backup.http.base_url"));
  }

  #[test]
  fn provider_api_key_is_required() {
    let result = serde_json::from_value::<Provider>(serde_json::json!({
      "name": "backup",
      "kind": "openai"
    }));

    assert!(result.is_err());
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
      "providers": {
        "openai": {
          "name": "openai",
          "kind": "openai"
        }
      }
    }));

    assert!(
      result
        .expect_err("missing API key must fail deserialization")
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
