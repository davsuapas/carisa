//! Supported provider kinds for runtime configuration.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Identifier for the provider family used by the runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProviderKind {
  /// `Anthropic` provider.
  Anthropic,
  /// `Azure` provider.
  Azure,
  /// `OpenAI` ChatGPT-style provider.
  ChatGpt,
  /// `Cohere` provider.
  Cohere,
  /// `GitHub Copilot` provider.
  Copilot,
  /// `DeepSeek` provider.
  DeepSeek,
  /// `DoubleWord` provider.
  DoubleWord,
  /// `Google Gemini` provider.
  Gemini,
  /// `Groq` provider.
  Groq,
  /// `Hugging Face` provider.
  HuggingFace,
  /// `Hyperbolic` provider.
  Hyperbolic,
  /// `Llamafile` provider.
  Llamafile,
  /// `Minimax` provider.
  Minimax,
  /// `Mira` provider.
  Mira,
  /// `Mistral` provider.
  Mistral,
  /// `Moonshot` provider.
  Moonshot,
  /// `Ollama` provider.
  Ollama,
  /// `OpenAI` provider.
  OpenAI,
  /// `OpenAI`-compatible provider.
  OpenAICompatible,
  /// `OpenRouter` provider.
  OpenRouter,
  /// `Perplexity` provider.
  Perplexity,
  /// `Together` provider.
  Together,
  /// `Venice` provider.
  Venice,
  /// `VoyageAI` provider.
  VoyageAI,
  /// `xAI` provider.
  Xai,
  /// `Xiaomi Mimo` provider.
  XiaomiMimo,
  /// `Z.ai` provider.
  Zai,
}

impl ProviderKind {
  /// Returns the provider name as a stable identifier used in config files.
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Anthropic => "anthropic",
      Self::Azure => "azure",
      Self::ChatGpt => "chatgpt",
      Self::Cohere => "cohere",
      Self::Copilot => "copilot",
      Self::DeepSeek => "deepseek",
      Self::DoubleWord => "doubleword",
      Self::Gemini => "gemini",
      Self::Groq => "groq",
      Self::HuggingFace => "huggingface",
      Self::Hyperbolic => "hyperbolic",
      Self::Llamafile => "llamafile",
      Self::Minimax => "minimax",
      Self::Mira => "mira",
      Self::Mistral => "mistral",
      Self::Moonshot => "moonshot",
      Self::Ollama => "ollama",
      Self::OpenAI => "openai",
      Self::OpenAICompatible => "openai_compatible",
      Self::OpenRouter => "openrouter",
      Self::Perplexity => "perplexity",
      Self::Together => "together",
      Self::Venice => "venice",
      Self::VoyageAI => "voyageai",
      Self::Xai => "xai",
      Self::XiaomiMimo => "xiaomimimo",
      Self::Zai => "zai",
    }
  }

  /// Returns `true` when the provider kind should include an API key secret.
  pub const fn requires_auth(self) -> bool {
    !matches!(
      self,
      Self::Ollama
        | Self::Llamafile
        | Self::OpenAICompatible
        | Self::Azure
        | Self::ChatGpt
        | Self::Copilot
    )
  }

  /// Returns `true` when the provider kind requires a base URL.
  pub const fn requires_base_url(self) -> bool {
    matches!(self, Self::OpenAICompatible | Self::Azure)
  }

  /// Returns `true` when the provider kind supports `tool_choice`.
  pub const fn supports_tool_choice(self) -> bool {
    matches!(
      self,
      Self::Anthropic
        | Self::Gemini
        | Self::Cohere
        | Self::OpenAI
        | Self::OpenAICompatible
        | Self::ChatGpt
    )
  }
}

impl From<&str> for ProviderKind {
  fn from(value: &str) -> Self {
    match value {
      "anthropic" => Self::Anthropic,
      "azure" => Self::Azure,
      "chatgpt" => Self::ChatGpt,
      "cohere" => Self::Cohere,
      "copilot" => Self::Copilot,
      "deepseek" => Self::DeepSeek,
      "doubleword" => Self::DoubleWord,
      "gemini" => Self::Gemini,
      "groq" => Self::Groq,
      "huggingface" => Self::HuggingFace,
      "hyperbolic" => Self::Hyperbolic,
      "llamafile" => Self::Llamafile,
      "minimax" => Self::Minimax,
      "mira" => Self::Mira,
      "mistral" => Self::Mistral,
      "moonshot" => Self::Moonshot,
      "ollama" => Self::Ollama,
      "openai_compatible" => Self::OpenAICompatible,
      "openrouter" => Self::OpenRouter,
      "perplexity" => Self::Perplexity,
      "together" => Self::Together,
      "venice" => Self::Venice,
      "voyageai" => Self::VoyageAI,
      "xai" => Self::Xai,
      "xiaomimimo" => Self::XiaomiMimo,
      "zai" => Self::Zai,
      _ => Self::OpenAI,
    }
  }
}

impl Serialize for ProviderKind {
  fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
  where
    S: Serializer,
  {
    serializer.serialize_str(self.as_str())
  }
}

impl<'de> Deserialize<'de> for ProviderKind {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: Deserializer<'de>,
  {
    let value = String::deserialize(deserializer)?;
    Ok(Self::from(value.as_str()))
  }
}
