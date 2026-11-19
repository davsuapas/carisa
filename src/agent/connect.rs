//! The single boundary between this crate and `rig-core`.
//!
//! Everything that has to know about rig lives here:
//!
//! - `ProviderId`, re-exported as [`crate::agent::ProviderId`];
//! - turning a [`Provider`] plus the shared [`Transport`] into a configured
//!   `reqwest` client wrapped in rig's erased HTTP handle;
//! - building the completion model from a provider selection and a model
//!   identifier;
//! - building a `CompletionRequest` from a [`ResolvedModel`];
//! - checking a resolved model against the catalog rig ships.
//!
//! Keeping the dependency here is a deliberate, local coupling: upgrading or
//! forking rig is a change confined to this file. Two rules keep it that
//! way: configure rig objects through its setters rather than by naming
//! struct literals, and match on rig enums only where a value has to be
//! split. The one exception is `CallerIdentity`, which rig exposes as a
//! plain struct with no setter.
//!
//! # Transport
//!
//! Each provider gets its own client because TLS settings are per provider
//! while timeouts and the proxy are shared. Headers are applied as transport
//! middleware, provider headers overriding [`Transport::default_headers`].
//!
//! # Secrets
//!
//! [`Secret::resolve`](crate::agent::Secret::resolve) happens here, when the
//! client is built, never while configuration is parsed: a platform can be
//! constructed without the environment being complete.
//!
//! # Retries
//!
//! [`Retries`](crate::agent::config::Retries) is *not* a middleware: rig's
//! middleware observes, it does not re-send. The policy is configuration the
//! executor's loop consumes.

use std::{collections::HashMap, fs, str::FromStr, time::Duration};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use rig_core::{
  catalog::Catalog,
  completion::{CompletionRequest, GenerationOptions, Message, OnUnsupported},
  driver::DynModel,
  http_client::{
    DynHttpClient, Exchange, HttpMiddleware, Result as HttpResult,
  },
  operation::Completion,
  providers::{
    openai::wire::CallerIdentity as RigCallerIdentity,
    registry::{Format, ProviderConfig},
  },
  wasm_compat::WasmBoxedFuture,
  wire::Secret as RigSecret,
};
use rig_reqwest::{ReqwestClient, reqwest};

use super::{
  config::{CallerIdentity, Model, Provider, Transport},
  error::PlatformError,
  resolve::ResolvedModel,
};

pub use rig_core::providers::registry::ProviderId;

/// Why a provider selection is not usable: the raw value is not a provider
/// this build registers.
///
/// `kind` keeps its JSON name and gains `vendor/format` for vendors that
/// register more than one protocol family. `openai_compatible` no longer
/// exists: a gateway that reproduces the `OpenAI` API is `openai` plus a
/// `base_url`.
///
/// # Errors
///
/// [`PlatformError::Invalid`] at `path` naming the unknown, ambiguous, or
/// unregistered selection and the alternatives rig offers.
pub(crate) fn resolve_provider_kind(
  path: &str,
  raw: &str,
) -> Result<ProviderId, PlatformError> {
  if raw == "openai_compatible" {
    return Err(PlatformError::invalid(
      path,
      "`openai_compatible` was removed; use `kind: \"openai\"` with a \
       provider `base_url`",
    ));
  }

  ProviderId::resolve(raw)
    .map_err(|error| PlatformError::invalid(path, error.to_string()))
}

/// Whether `kind` speaks the `OpenAI` format, the only family whose
/// configuration carries a caller identity.
pub(crate) fn speaks_openai(kind: &ProviderId) -> bool {
  kind.format() == Some(Format::OpenAi)
}

/// Checks that every header name and value is a legal HTTP header.
///
/// # Errors
///
/// [`PlatformError::Invalid`] at `path` naming the offending header.
pub(crate) fn validate_headers(
  path: &str,
  headers: &HashMap<String, String>,
) -> Result<(), PlatformError> {
  insert_headers(&mut HeaderMap::new(), path, headers)
}

/// Builds the completion model a resolved agent calls.
///
/// This is the entry point for the executor: configuration is resolved into
/// a [`ResolvedModel`], and this turns it into a model bound to its own
/// client.
///
/// # Errors
///
/// [`PlatformError::Invalid`] when the provider is not configurable by this
/// build, the credential cannot be resolved, the client cannot be built, or
/// a header is malformed.
pub fn completion_model(
  resolved: &ResolvedModel<'_>,
  transport: Option<&Transport>,
) -> Result<DynModel<Completion>, PlatformError> {
  model_for(resolved.provider(), resolved.model(), transport)
}

/// Builds the completion model for `model`, reached through `provider`.
fn model_for(
  provider: &Provider,
  model: &Model,
  transport: Option<&Transport>,
) -> Result<DynModel<Completion>, PlatformError> {
  let path = format!("providers.{}", provider.name());
  let secret = match provider.api_key() {
    Some(secret) => secret.resolve()?,
    None => RigSecret::from(String::new()),
  };
  let config = provider.kind().config(secret).ok_or_else(|| {
    PlatformError::invalid(
      format!("{path}.kind"),
      format!("`{}` is not configurable by this build", provider.kind()),
    )
  })?;
  let config = match provider.base_url() {
    Some(base_url) => config.with_base_url(base_url.as_str()),
    None => config,
  };
  let config = with_identity(config, transport.and_then(Transport::identity));
  let http = build_http(provider, transport)?;
  let model_id = model.id();

  // The one exhaustive match on a rig enum, and the reason rule 16 allows
  // it: each variant carries a different configuration type, and each one
  // builds its model through its own `connect`/`completion` pair. A variant
  // rig adds later fails to compile here instead of silently going
  // unhandled.
  match config {
    ProviderConfig::OpenAi(config) => {
      Ok(config.connect(http).completion(model_id).erase())
    }
    ProviderConfig::Anthropic(config) => {
      Ok(config.connect(http).completion(model_id).erase())
    }
    ProviderConfig::Gemini(config) => {
      Ok(config.connect(http).completion(model_id).erase())
    }
  }
}

/// Builds the request a resolved model runs with.
///
/// Temperature and the token budget ride on the request itself; `top_p`,
/// `seed`, and the rest of the portable knobs ride on `GenerationOptions`.
/// The unsupported-option policy is fixed to `Error`: configuration cannot
/// downgrade a refusal into a silent drop.
pub(crate) fn request_for(
  resolved: &ResolvedModel<'_>,
  prompt: impl Into<Message>,
) -> CompletionRequest {
  let generation = resolved.generation();
  let mut options =
    GenerationOptions::new().on_unsupported(OnUnsupported::Error);
  if let Some(top_p) = generation.top_p() {
    options = options.top_p(top_p);
  }
  if let Some(seed) = generation.seed() {
    options = options.seed(seed);
  }

  let mut request = CompletionRequest::new(prompt)
    .temperature(generation.temperature())
    .max_tokens(generation.max_tokens().map(u64::from))
    .options(options);

  if !generation.additional_params().is_empty() {
    request = request.additional_params(serde_json::Value::Object(
      generation
        .additional_params()
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect(),
    ));
  }

  request
}

/// Checks `resolved` against the catalog rig ships.
///
/// A model the catalog does not list is not an error: local servers and
/// corporate gateways legitimately serve identifiers rig never heard of. A
/// model it does list is checked with the request the platform would send,
/// so `reasoning`, `cache`, `top_p`, and `temperature` are all validated.
/// Refusals are reported, never silently corrected.
///
/// # Errors
///
/// [`PlatformError::Invalid`] at `models.<id>.<option>` with rig's reason.
pub(crate) fn validate_against_catalog(
  resolved: &ResolvedModel<'_>,
) -> Result<(), PlatformError> {
  let model_id = resolved.model().id();
  let reference =
    format!("{}/{}", resolved.provider().kind().vendor(), model_id);
  let Ok(found) = Catalog::builtin().resolve(&reference) else {
    return Ok(());
  };

  let request = request_for(resolved, "carisa schema validation");
  found
    .spec
    .refusals(&request)
    .first()
    .map_or(Ok(()), |refusal| {
      Err(PlatformError::invalid(
        format!("models.{model_id}.{}", refusal.option),
        refusal.reason.clone(),
      ))
    })
}

/// Applies the configured caller identity, if any.
fn with_identity(
  config: ProviderConfig,
  identity: Option<&CallerIdentity>,
) -> ProviderConfig {
  let Some(identity) = identity else {
    return config;
  };

  // rig has no setter for `identity`, so this is the one place a rig struct
  // is named literally. See the module documentation.
  match config {
    ProviderConfig::OpenAi(mut config) => {
      config.identity = Some(RigCallerIdentity {
        originator: identity.originator().to_owned(),
        user_agent: identity
          .user_agent()
          .cloned()
          .unwrap_or_else(default_user_agent),
      });
      ProviderConfig::OpenAi(config)
    }
    other => other,
  }
}

/// The `user-agent` sent when the identity does not override it.
fn default_user_agent() -> String {
  format!("{}/{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

/// Builds one client for `provider` from the shared transport and its TLS
/// settings.
fn build_http(
  provider: &Provider,
  transport: Option<&Transport>,
) -> Result<DynHttpClient, PlatformError> {
  let path = format!("providers.{}", provider.name());
  let mut builder = reqwest::Client::builder();

  if let Some(transport) = transport {
    if let Some(timeout_ms) = transport.timeout_ms() {
      builder = builder.timeout(Duration::from_millis(timeout_ms));
    }
    if let Some(timeout_ms) = transport.connect_timeout_ms() {
      builder = builder.connect_timeout(Duration::from_millis(timeout_ms));
    }
    if let Some(proxy) = transport.proxy() {
      builder = builder.proxy(reqwest::Proxy::all(proxy.as_str()).map_err(
        |error| PlatformError::Url {
          path: "transport.proxy".to_owned(),
          message: error.to_string(),
        },
      )?);
    }
  }

  if let Some(tls) = provider.tls() {
    if tls.insecure_skip_verify() {
      builder = builder.danger_accept_invalid_certs(true);
    }
    if let Some(ca_file) = tls.ca_file() {
      let ca_path = format!("{path}.tls.ca_file");
      let pem = fs::read(ca_file).map_err(|source| PlatformError::Io {
        path: ca_path.clone(),
        source,
      })?;
      for certificate in reqwest::Certificate::from_pem_bundle(&pem)
        .map_err(|error| PlatformError::invalid(&ca_path, error.to_string()))?
      {
        builder = builder.add_root_certificate(certificate);
      }
    }
  }

  let client = builder.build().map_err(|error| {
    PlatformError::invalid(&path, format!("HTTP client: {error}"))
  })?;

  let mut http = DynHttpClient::new(ReqwestClient::from(client));
  let headers = all_headers(provider, transport)?;
  if !headers.is_empty() {
    http = http.with_middleware(HeaderMiddleware { headers });
  }
  Ok(http)
}

/// Every header a provider's requests carry: transport defaults first, then
/// the provider's own, which win.
fn all_headers(
  provider: &Provider,
  transport: Option<&Transport>,
) -> Result<HeaderMap, PlatformError> {
  let mut headers = HeaderMap::new();
  if let Some(transport) = transport {
    insert_headers(
      &mut headers,
      "transport.default_headers",
      transport.default_headers(),
    )?;
  }
  insert_headers(
    &mut headers,
    &format!("providers.{}.headers", provider.name()),
    provider.headers(),
  )?;
  Ok(headers)
}

/// Inserts configured header pairs into `target`, validating each one.
fn insert_headers(
  target: &mut HeaderMap,
  path: &str,
  headers: &HashMap<String, String>,
) -> Result<(), PlatformError> {
  for (name, value) in headers {
    let header_name = HeaderName::from_str(name).map_err(|error| {
      PlatformError::invalid(
        path,
        format!("invalid header name `{name}`: {error}"),
      )
    })?;
    let header_value = HeaderValue::from_str(value).map_err(|error| {
      PlatformError::invalid(
        path,
        format!("invalid value for header `{name}`: {error}"),
      )
    })?;
    target.insert(header_name, header_value);
  }
  Ok(())
}

/// Adds a fixed header set to every request a provider makes.
struct HeaderMiddleware {
  headers: HeaderMap,
}

impl HttpMiddleware for HeaderMiddleware {
  fn before_request_headers<'a>(
    &'a self,
    _exchange: Exchange<'a>,
    headers: &'a mut HeaderMap,
  ) -> WasmBoxedFuture<'a, HttpResult<()>> {
    Box::pin(async move {
      for (name, value) in &self.headers {
        headers.insert(name, value.clone());
      }
      Ok(())
    })
  }
}

#[cfg(test)]
mod tests {
  use std::sync::Arc;

  use super::*;
  use crate::agent::{
    AgentBuilder, AgentModel, AgentPlatform, AgentPlatformBuilder, Secret,
    config::{
      Generation, GenerationBuilder, ModelBuilder, ProviderBuilder,
      TransportBuilder,
    },
  };
  use crate::memory::session::inmemory::InMemoryLogStorage;

  fn provider(name: &str, kind: &str) -> ProviderBuilder {
    ProviderBuilder::default()
      .name(name.to_owned())
      .kind(resolve_provider_kind("kind", kind).expect("kind is registered"))
      .api_key(Secret::Literal("test-api-key".to_owned()))
  }

  fn resolved(generation: Generation) -> AgentPlatform {
    AgentPlatformBuilder::default()
      .providers(vec![
        provider("openai", "openai")
          .build()
          .expect("provider builder has all required fields"),
      ])
      .models(vec![
        ModelBuilder::default()
          .id("gpt-4o".to_owned())
          .provider("openai".to_owned())
          .generation(generation)
          .build()
          .expect("model builder has all required fields"),
      ])
      .default_model("gpt-4o")
      .log_storage(Arc::new(InMemoryLogStorage::new()))
      .build()
      .expect("platform is valid")
  }

  fn agent() -> crate::agent::Agent {
    AgentBuilder::default()
      .instructions("Instructions".to_owned())
      .model(AgentModel::new("gpt-4o"))
      .build()
      .expect("instructions provided")
  }

  #[test]
  fn unsupported_kind_is_reported_not_panicked() {
    let declared =
      ProviderId::declare("carisa-test-gateway").expect("name is valid");

    assert!(declared.config(RigSecret::from("key".to_owned())).is_none());

    let provider = Provider::new("gateway", declared, None);
    let model = Model::new("big", "gateway");
    let Err(error) = model_for(&provider, &model, None) else {
      panic!("a declared provider cannot be configured");
    };

    assert!(error.to_string().contains("providers.gateway.kind"));
  }

  #[test]
  fn client_is_built_from_transport_and_tls() {
    let transport = TransportBuilder::default()
      .timeout_ms(1000)
      .connect_timeout_ms(500)
      .proxy(url::Url::parse("http://proxy.example:3128").unwrap())
      .default_headers(HashMap::from([(
        "x-client".to_owned(),
        "carisa".to_owned(),
      )]))
      .build()
      .expect("no required fields");
    let provider = provider("openai", "openai")
      .tls(
        crate::agent::config::TlsBuilder::default()
          .insecure_skip_verify(true)
          .build()
          .expect("no required fields"),
      )
      .headers(HashMap::from([(
        "x-provider".to_owned(),
        "gateway".to_owned(),
      )]))
      .build()
      .expect("provider builder has all required fields");

    assert!(build_http(&provider, Some(&transport)).is_ok());
  }

  #[test]
  fn unreadable_ca_bundle_is_an_error() {
    let provider = provider("openai", "openai")
      .tls(
        crate::agent::config::TlsBuilder::default()
          .ca_file("/nonexistent/carisa-ca.pem")
          .build()
          .expect("no required fields"),
      )
      .build()
      .expect("provider builder has all required fields");
    let Err(error) = build_http(&provider, None) else {
      panic!("the bundle cannot be read");
    };

    assert!(error.to_string().contains("providers.openai.tls.ca_file"));
  }

  #[test]
  fn invalid_header_is_an_error() {
    let error = validate_headers(
      "providers.openai.headers",
      &HashMap::from([("bad header".to_owned(), "value".to_owned())]),
    )
    .expect_err("a space is not a legal header name");

    assert!(error.to_string().contains("providers.openai.headers"));
  }

  #[test]
  fn request_carries_the_expected_generation_settings() {
    let platform = resolved(
      GenerationBuilder::default()
        .temperature(0.3)
        .max_tokens(512)
        .top_p(0.8)
        .seed(7)
        .additional_params(HashMap::from([(
          "reasoning_effort".to_owned(),
          serde_json::json!("high"),
        )]))
        .build()
        .expect("no required fields"),
    );
    let resolved = platform.resolve(&agent()).expect("model resolves");
    let request = request_for(&resolved, "hello");

    assert_eq!(request.temperature, Some(0.3));
    assert_eq!(request.max_tokens, Some(512));
    assert_eq!(request.options.top_p, Some(0.8));
    assert_eq!(request.options.seed, Some(7));
    assert_eq!(request.options.unsupported_policy(), OnUnsupported::Error);
    assert_eq!(
      request
        .additional_params
        .as_ref()
        .and_then(|params| params.get("reasoning_effort")),
      Some(&serde_json::json!("high"))
    );
  }

  #[test]
  fn catalog_accepts_a_supported_temperature() {
    let platform = resolved(
      GenerationBuilder::default()
        .temperature(0.3)
        .build()
        .expect("no required fields"),
    );
    let resolved = platform.resolve(&agent()).expect("model resolves");

    assert!(validate_against_catalog(&resolved).is_ok());
  }

  #[test]
  fn catalog_ignores_a_model_it_does_not_list() {
    let platform = AgentPlatformBuilder::default()
      .providers(vec![
        provider("openai", "openai")
          .build()
          .expect("provider builder has all required fields"),
      ])
      .models(vec![Model::new("carisa-unlisted-model", "openai")])
      .default_model("carisa-unlisted-model")
      .log_storage(Arc::new(InMemoryLogStorage::new()))
      .build()
      .expect("platform is valid");
    let agent = AgentBuilder::default()
      .instructions("Instructions".to_owned())
      .build()
      .expect("instructions provided");
    let resolved = platform.resolve(&agent).expect("model resolves");

    assert!(validate_against_catalog(&resolved).is_ok());
  }

  #[test]
  fn identity_is_only_applied_to_openai_configurations() {
    let identity = CallerIdentity::new("carisa");
    let config = resolve_provider_kind("kind", "anthropic")
      .expect("anthropic is registered")
      .config(RigSecret::from("key".to_owned()))
      .expect("registered");

    assert_eq!(with_identity(config.clone(), Some(&identity)), config);
  }
}
