//! Model routing — maps model classes to specific provider + model pairs.

use serde::{Deserialize, Serialize};

use crate::ModelClass;

/// A specific model on a specific provider backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSpec {
    /// Provider backend: "bedrock", "openai", or "anthropic"
    pub provider: String,
    /// Model identifier (e.g., "anthropic.claude-3-5-sonnet-20241022-v2:0" for Bedrock,
    /// "gpt-4o" for OpenAI, "claude-sonnet-4-6" for Anthropic)
    pub model: String,
}

impl ModelSpec {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }

    /// Parse a "provider/model" string into a ModelSpec.
    /// If no slash, defaults to "anthropic" provider.
    pub fn parse(spec: &str) -> Self {
        if let Some((provider, model)) = spec.split_once('/') {
            Self::new(provider, model)
        } else {
            Self::new("anthropic", spec)
        }
    }
}

/// Maps each model class to a specific provider + model.
///
/// Configurable via TOML or environment variables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub small: ModelSpec,
    pub medium: ModelSpec,
    pub large: ModelSpec,
}

impl ModelConfig {
    /// Resolve a model class to its configured ModelSpec.
    pub fn resolve(&self, class: ModelClass) -> &ModelSpec {
        match class {
            ModelClass::Small => &self.small,
            ModelClass::Medium => &self.medium,
            ModelClass::Large => &self.large,
        }
    }
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            small: ModelSpec::new("bedrock", "anthropic.claude-3-5-haiku-20241022-v1:0"),
            medium: ModelSpec::new("bedrock", "anthropic.claude-3-5-sonnet-20241022-v2:0"),
            large: ModelSpec::new("openai", "gpt-4o"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_spec_new() {
        let spec = ModelSpec::new("bedrock", "anthropic.claude-3-5-sonnet-20241022-v2:0");
        assert_eq!(spec.provider, "bedrock");
        assert_eq!(spec.model, "anthropic.claude-3-5-sonnet-20241022-v2:0");
    }

    #[test]
    fn test_model_spec_parse_with_provider() {
        let spec = ModelSpec::parse("openai/gpt-4o");
        assert_eq!(spec.provider, "openai");
        assert_eq!(spec.model, "gpt-4o");
    }

    #[test]
    fn test_model_spec_parse_without_provider() {
        let spec = ModelSpec::parse("claude-sonnet-4-6");
        assert_eq!(spec.provider, "anthropic");
        assert_eq!(spec.model, "claude-sonnet-4-6");
    }

    #[test]
    fn test_model_config_default() {
        let config = ModelConfig::default();
        assert_eq!(config.small.provider, "bedrock");
        assert_eq!(config.medium.provider, "bedrock");
        assert_eq!(config.large.provider, "openai");
    }

    #[test]
    fn test_model_config_resolve() {
        let config = ModelConfig::default();
        let small = config.resolve(ModelClass::Small);
        assert_eq!(small.provider, "bedrock");
        assert!(small.model.contains("haiku"));

        let large = config.resolve(ModelClass::Large);
        assert_eq!(large.provider, "openai");
        assert_eq!(large.model, "gpt-4o");
    }

    #[test]
    fn test_model_spec_serde_roundtrip() {
        let spec = ModelSpec::new("bedrock", "anthropic.claude-3-5-sonnet-20241022-v2:0");
        let json = serde_json::to_string(&spec).unwrap();
        let parsed: ModelSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.provider, spec.provider);
        assert_eq!(parsed.model, spec.model);
    }

    #[test]
    fn test_model_config_serde_roundtrip() {
        let config = ModelConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let parsed: ModelConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.small.provider, config.small.provider);
        assert_eq!(parsed.large.model, config.large.model);
    }
}
