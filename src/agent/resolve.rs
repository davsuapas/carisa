//! Resolution of an agent to the provider, model, and generation it uses.
//!
//! Resolution is where platform precedence is applied, once, so neither the
//! executor nor the caller has to repeat it:
//!
//! 1. The model is the agent's [`AgentModel::id`], or the platform default
//!    when the agent selects none.
//! 2. The provider is the model's `provider` reference.
//! 3. The generation is the model's baseline with the agent's overrides laid
//!    over it, field by field. A field the agent leaves unset is inherited;
//!    `additional_params` is merged by key, with the agent winning.
//!
//! [`AgentPlatform::validate`](crate::agent::AgentPlatform::validate) walks
//! every agent through this module, so a broken reference fails at startup
//! rather than on the first call.
//!
//! ```rust,no_run
//! use carisa_core::{AgentBuilder, AgentPlatform};
//!
//! # fn example(platform: &AgentPlatform) -> Result<(), carisa_core::PlatformError> {
//! let agent = AgentBuilder::default()
//!   .instructions("Answer concisely.".to_owned())
//!   .build()
//!   .expect("instructions are set");
//! let resolved = platform.resolve(&agent)?;
//!
//! assert_eq!(resolved.model().id(), "gpt-4o");
//! # Ok(())
//! # }
//! ```
//!
//! [`AgentModel::id`]: crate::agent::AgentModel::id

use crate::agent::{
  Agent, AgentPlatform, PlatformError,
  config::{Generation, Model, Provider},
};

/// A model with the provider and generation it resolves to.
///
/// The provider and model are borrowed from the platform; the generation is
/// owned because it is the result of merging them.
#[derive(Debug, Clone)]
pub struct ResolvedModel<'a> {
  provider: &'a Provider,
  model: &'a Model,
  generation: Generation,
}

impl<'a> ResolvedModel<'a> {
  /// Pairs `provider` with `model` and the already merged `generation`.
  pub(crate) const fn new(
    provider: &'a Provider,
    model: &'a Model,
    generation: Generation,
  ) -> Self {
    Self {
      provider,
      model,
      generation,
    }
  }

  /// Returns the provider that serves the model.
  pub const fn provider(&self) -> &'a Provider {
    self.provider
  }

  /// Returns the resolved model definition.
  pub const fn model(&self) -> &'a Model {
    self.model
  }

  /// Returns the merged generation settings.
  pub const fn generation(&self) -> &Generation {
    &self.generation
  }
}

impl AgentPlatform {
  /// Resolves the model, provider, and generation an agent runs with.
  ///
  /// # Errors
  ///
  /// [`PlatformError::Invalid`] when the agent names a model that is not
  /// configured, when no default model exists and the agent names none, or
  /// when the model's provider is not configured.
  pub fn resolve(
    &self,
    agent: &Agent,
  ) -> Result<ResolvedModel<'_>, PlatformError> {
    self.resolve_at(agent, "agent.model.id")
  }

  /// [`Self::resolve`] with the path validation reports on failure.
  pub(crate) fn resolve_at<'a>(
    &'a self,
    agent: &Agent,
    path: &str,
  ) -> Result<ResolvedModel<'a>, PlatformError> {
    let Some(requested) = agent.model() else {
      let default = self.defaults().model().ok_or_else(|| {
        PlatformError::invalid(
          "defaults.model",
          "no default model is configured",
        )
      })?;
      let model = self
        .models()
        .iter()
        .find(|model| model.id() == default)
        .ok_or_else(|| {
          PlatformError::invalid(
            "defaults.model",
            format!("model `{default}` is not configured"),
          )
        })?;
      return self.without_overrides(model);
    };

    let model = self
      .models()
      .iter()
      .find(|model| model.id() == requested.id())
      .ok_or_else(|| {
        PlatformError::invalid(
          path,
          format!("model `{}` is not configured", requested.id()),
        )
      })?;

    let mut generation = model.generation().clone();
    generation.overlay(requested.generation());
    generation.overlay_params(requested.additional_params());

    self.finish(model, generation)
  }

  /// Resolves `model` as the default does: with no overrides.
  fn without_overrides<'a>(
    &'a self,
    model: &'a Model,
  ) -> Result<ResolvedModel<'a>, PlatformError> {
    let generation = model.generation().clone();
    self.finish(model, generation)
  }

  /// Pairs `model` with its provider and the merged `generation`.
  fn finish<'a>(
    &'a self,
    model: &'a Model,
    generation: Generation,
  ) -> Result<ResolvedModel<'a>, PlatformError> {
    let provider = self
      .providers()
      .iter()
      .find(|provider| provider.name() == model.provider())
      .ok_or_else(|| {
        PlatformError::invalid(
          format!("models.{}.provider", model.id()),
          format!("provider `{}` is not configured", model.provider()),
        )
      })?;

    Ok(ResolvedModel {
      provider,
      model,
      generation,
    })
  }
}

#[cfg(test)]
mod tests {
  use std::collections::HashMap;
  use std::sync::Arc;

  use super::*;
  use crate::agent::{
    AgentModel, ProviderId, Secret,
    config::{GenerationBuilder, ProviderBuilder, TransportBuilder},
    platform::AgentPlatformBuilder,
    types::AgentModelBuilder,
  };
  use crate::memory::session::inmemory::InMemoryLogStorage;

  fn provider(name: &str) -> Provider {
    ProviderBuilder::default()
      .name(name.to_owned())
      .kind(ProviderId::resolve("openai").expect("openai is registered"))
      .api_key(Secret::Literal("test-api-key".to_owned()))
      .build()
      .expect("provider builder has all required fields")
  }

  fn generation() -> GenerationBuilder {
    GenerationBuilder::default()
      .temperature(0.2)
      .max_tokens(2048_u32)
      .additional_params(HashMap::from([
        ("reasoning_effort".to_owned(), serde_json::json!("low")),
        ("verbosity".to_owned(), serde_json::json!("low")),
      ]))
  }

  fn platform(default_model: Option<&str>) -> AgentPlatform {
    let mut builder = AgentPlatformBuilder::default()
      .providers(vec![provider("openai")])
      .models(vec![
        crate::agent::config::ModelBuilder::default()
          .id("gpt-4o".to_owned())
          .provider("openai".to_owned())
          .generation(generation().build().expect("no required fields"))
          .build()
          .expect("model builder has all required fields"),
        Model::new("gpt-4o-mini", "openai"),
      ])
      .transport(TransportBuilder::default().build().expect("no fields"))
      .log_storage(Arc::new(InMemoryLogStorage::new()));
    if let Some(default_model) = default_model {
      builder = builder.default_model(default_model);
    }
    builder.build().expect("platform is valid")
  }

  fn agent(model: Option<AgentModel>) -> Agent {
    let mut builder = crate::agent::AgentBuilder::default();
    builder.instructions("Instructions".to_owned());
    if let Some(model) = model {
      builder.model(model);
    }
    builder.build().expect("instructions are provided")
  }

  #[test]
  fn explicit_model_wins_over_the_default() {
    let platform = platform(Some("gpt-4o"));
    let resolved = platform
      .resolve(&agent(Some(AgentModel::new("gpt-4o-mini"))))
      .expect("the model exists");

    assert_eq!(resolved.model().id(), "gpt-4o-mini");
    assert_eq!(resolved.provider().name(), "openai");
  }

  #[test]
  fn agent_without_a_model_uses_the_default() {
    let platform = platform(Some("gpt-4o"));
    let resolved = platform.resolve(&agent(None)).expect("default resolves");

    assert_eq!(resolved.model().id(), "gpt-4o");
    assert_eq!(resolved.generation(), &generation().build().unwrap());
  }

  #[test]
  fn agent_overrides_only_the_fields_it_sets() {
    let platform = platform(Some("gpt-4o"));
    let overrides = AgentModelBuilder::default()
      .id("gpt-4o".to_owned())
      .generation(
        GenerationBuilder::default()
          .temperature(0.9)
          .build()
          .expect("no required fields"),
      )
      .build()
      .expect("model identifier provided");
    let resolved = platform
      .resolve(&agent(Some(overrides)))
      .expect("the model exists");

    assert_eq!(resolved.generation().temperature(), Some(0.9));
    assert_eq!(resolved.generation().max_tokens(), Some(2048));
  }

  #[test]
  fn additional_params_merge_by_key_with_the_agent_winning() {
    let platform = platform(Some("gpt-4o"));
    let overrides = AgentModelBuilder::default()
      .id("gpt-4o".to_owned())
      .additional_params(HashMap::from([(
        "verbosity".to_owned(),
        serde_json::json!("high"),
      )]))
      .build()
      .expect("model identifier provided");
    let resolved = platform
      .resolve(&agent(Some(overrides)))
      .expect("the model exists");
    let params = resolved.generation().additional_params();

    assert_eq!(params.get("verbosity"), Some(&serde_json::json!("high")));
    assert_eq!(
      params.get("reasoning_effort"),
      Some(&serde_json::json!("low"))
    );
  }

  #[test]
  fn unknown_model_fails_with_its_path() {
    let platform = platform(Some("gpt-4o"));
    let error = platform
      .resolve(&agent(Some(AgentModel::new("missing"))))
      .expect_err("the model is not configured");

    assert_eq!(
      error.to_string(),
      "invalid value at `agent.model.id`: model `missing` is not configured"
    );
  }

  #[test]
  fn missing_default_fails_when_the_agent_names_no_model() {
    let platform = platform(None);
    let error = platform
      .resolve(&agent(None))
      .expect_err("no default model exists");

    assert!(error.to_string().contains("defaults.model"));
  }
}
