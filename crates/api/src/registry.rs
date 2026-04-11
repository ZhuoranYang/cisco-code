//! Provider registry — factory for instantiating LLM providers by name.
//!
//! Matches Codex's model provider info pattern: providers are configured
//! with name, base URL, env key, and wire API. The registry creates
//! Provider instances from configuration.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::bedrock::BedrockClient;
use crate::client::{AnthropicClient, Provider};
use crate::openai::OpenAIClient;

/// Provider type identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    Anthropic,
    OpenAI,
    Bedrock,
    Azure,
    /// Any OpenAI-compatible endpoint (Groq, Together, Ollama, etc.)
    OpenAICompatible,
}

/// Configuration for a model provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub provider_type: ProviderType,
    /// Base URL for the API.
    pub base_url: Option<String>,
    /// Environment variable name for the API key.
    pub env_key: Option<String>,
    /// Explicit API key (use env_key in production).
    pub api_key: Option<String>,
    /// AWS region (for Bedrock).
    pub aws_region: Option<String>,
    /// Maximum retries for requests.
    pub max_retries: Option<u32>,
    /// Custom headers to include in requests.
    pub headers: Option<HashMap<String, String>>,
}

/// Azure OpenAI endpoint detection.
pub fn is_azure_endpoint(base_url: &str) -> bool {
    let lower = base_url.to_ascii_lowercase();
    lower.contains("openai.azure.")
        || lower.contains("cognitiveservices.azure.")
        || lower.contains("aoai.azure.")
        || lower.contains("azure-api.")
        || lower.contains("azurefd.")
        || lower.contains("windows.net/openai")
}

/// Registry of available providers.
pub struct ProviderRegistry {
    configs: HashMap<String, ProviderConfig>,
    /// Default provider name.
    default: Option<String>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            configs: HashMap::new(),
            default: None,
        }
    }

    /// Create with default providers configured from environment.
    pub fn from_env() -> Self {
        let mut registry = Self::new();

        // Anthropic (from ANTHROPIC_API_KEY)
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            registry.register(ProviderConfig {
                name: "anthropic".into(),
                provider_type: ProviderType::Anthropic,
                base_url: std::env::var("ANTHROPIC_BASE_URL").ok(),
                env_key: Some("ANTHROPIC_API_KEY".into()),
                api_key: Some(key),
                aws_region: None,
                max_retries: None,
                headers: None,
            });
            if registry.default.is_none() {
                registry.default = Some("anthropic".into());
            }
        }

        // OpenAI (from OPENAI_API_KEY or CODEX_API_KEY)
        let openai_key = std::env::var("OPENAI_API_KEY")
            .or_else(|_| std::env::var("CODEX_API_KEY"))
            .ok();
        if let Some(key) = openai_key {
            let base_url = std::env::var("OPENAI_BASE_URL")
                .or_else(|_| std::env::var("OPENAI_API_BASE"))
                .ok();
            let provider_type = base_url
                .as_deref()
                .map(|u| {
                    if is_azure_endpoint(u) {
                        ProviderType::Azure
                    } else {
                        ProviderType::OpenAI
                    }
                })
                .unwrap_or(ProviderType::OpenAI);

            registry.register(ProviderConfig {
                name: "openai".into(),
                provider_type,
                base_url,
                env_key: Some("OPENAI_API_KEY".into()),
                api_key: Some(key),
                aws_region: None,
                max_retries: None,
                headers: None,
            });
        }

        // AWS Bedrock (from AWS credentials)
        if std::env::var("AWS_ACCESS_KEY_ID").is_ok()
            || std::env::var("AWS_PROFILE").is_ok()
            || std::env::var("AWS_SESSION_TOKEN").is_ok()
        {
            registry.register(ProviderConfig {
                name: "bedrock".into(),
                provider_type: ProviderType::Bedrock,
                base_url: None,
                env_key: Some("AWS_ACCESS_KEY_ID".into()),
                api_key: None,
                aws_region: std::env::var("AWS_REGION")
                    .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
                    .ok(),
                max_retries: None,
                headers: None,
            });
        }

        registry
    }

    /// Register a provider configuration.
    pub fn register(&mut self, config: ProviderConfig) {
        let name = config.name.clone();
        self.configs.insert(name, config);
    }

    /// Set the default provider.
    pub fn set_default(&mut self, name: &str) {
        self.default = Some(name.to_string());
    }

    /// Get the default provider name.
    pub fn default_name(&self) -> Option<&str> {
        self.default.as_deref()
    }

    /// List available provider names.
    pub fn available(&self) -> Vec<&str> {
        self.configs.keys().map(|s| s.as_str()).collect()
    }

    /// Get a provider config by name.
    pub fn get_config(&self, name: &str) -> Option<&ProviderConfig> {
        self.configs.get(name)
    }

    /// Create a Provider instance from a named config.
    pub fn create(&self, name: &str) -> Result<Arc<dyn Provider>> {
        let config = self
            .configs
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown provider: {name}"))?;

        self.create_from_config(config)
    }

    /// Create the default provider.
    pub fn create_default(&self) -> Result<Arc<dyn Provider>> {
        let name = self
            .default
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("no default provider configured"))?;
        self.create(name)
    }

    /// Create a Provider from config.
    fn create_from_config(&self, config: &ProviderConfig) -> Result<Arc<dyn Provider>> {
        let api_key = config
            .api_key
            .clone()
            .or_else(|| {
                config
                    .env_key
                    .as_ref()
                    .and_then(|k| std::env::var(k).ok())
            })
            .unwrap_or_default();

        match config.provider_type {
            ProviderType::Anthropic => {
                if api_key.is_empty() {
                    bail!("Anthropic API key not found. Set ANTHROPIC_API_KEY.");
                }
                let mut client = AnthropicClient::new(&api_key);
                if let Some(url) = &config.base_url {
                    client = client.with_base_url(url);
                }
                Ok(Arc::new(client))
            }
            ProviderType::OpenAI | ProviderType::Azure | ProviderType::OpenAICompatible => {
                if api_key.is_empty() {
                    bail!("OpenAI API key not found. Set OPENAI_API_KEY.");
                }
                let mut client = OpenAIClient::new(&api_key);
                if let Some(url) = &config.base_url {
                    client = client.with_base_url(url);
                }
                Ok(Arc::new(client))
            }
            ProviderType::Bedrock => {
                let region = config
                    .aws_region
                    .clone()
                    .or_else(|| std::env::var("AWS_REGION").ok())
                    .unwrap_or_else(|| "us-east-1".to_string());
                // Use from_env() which reads AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, etc.
                match BedrockClient::from_env() {
                    Ok(client) => Ok(Arc::new(client)),
                    Err(_) => {
                        // Fall back to empty credentials — will fail on actual API calls
                        // but allows registry construction when env is partially configured
                        let access_key = std::env::var("AWS_ACCESS_KEY_ID").unwrap_or_default();
                        let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY").unwrap_or_default();
                        let mut client = BedrockClient::new(access_key, secret_key, &region);
                        if let Ok(token) = std::env::var("AWS_SESSION_TOKEN") {
                            client = client.with_session_token(token);
                        }
                        Ok(Arc::new(client))
                    }
                }
            }
        }
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Model metadata for routing and display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub supports_extended_thinking: bool,
}

/// Well-known model catalog.
pub fn builtin_models() -> Vec<ModelInfo> {
    vec![
        // Anthropic
        ModelInfo {
            id: "claude-opus-4-6".into(),
            display_name: "Claude Opus 4.6".into(),
            provider: "anthropic".into(),
            context_window: Some(200_000),
            max_output_tokens: Some(32_000),
            supports_tools: true,
            supports_vision: true,
            supports_extended_thinking: true,
        },
        ModelInfo {
            id: "claude-sonnet-4-6".into(),
            display_name: "Claude Sonnet 4.6".into(),
            provider: "anthropic".into(),
            context_window: Some(200_000),
            max_output_tokens: Some(16_000),
            supports_tools: true,
            supports_vision: true,
            supports_extended_thinking: true,
        },
        ModelInfo {
            id: "claude-haiku-4-5-20251001".into(),
            display_name: "Claude Haiku 4.5".into(),
            provider: "anthropic".into(),
            context_window: Some(200_000),
            max_output_tokens: Some(8_192),
            supports_tools: true,
            supports_vision: true,
            supports_extended_thinking: false,
        },
        // Bedrock (Anthropic models via AWS)
        ModelInfo {
            id: "anthropic.claude-sonnet-4-6-v1:0".into(),
            display_name: "Claude Sonnet 4.6 (Bedrock)".into(),
            provider: "bedrock".into(),
            context_window: Some(200_000),
            max_output_tokens: Some(16_000),
            supports_tools: true,
            supports_vision: true,
            supports_extended_thinking: true,
        },
        ModelInfo {
            id: "anthropic.claude-haiku-4-5-v1:0".into(),
            display_name: "Claude Haiku 4.5 (Bedrock)".into(),
            provider: "bedrock".into(),
            context_window: Some(200_000),
            max_output_tokens: Some(8_192),
            supports_tools: true,
            supports_vision: true,
            supports_extended_thinking: false,
        },
        // OpenAI
        ModelInfo {
            id: "gpt-4o".into(),
            display_name: "GPT-4o".into(),
            provider: "openai".into(),
            context_window: Some(128_000),
            max_output_tokens: Some(16_384),
            supports_tools: true,
            supports_vision: true,
            supports_extended_thinking: false,
        },
        ModelInfo {
            id: "gpt-4o-mini".into(),
            display_name: "GPT-4o Mini".into(),
            provider: "openai".into(),
            context_window: Some(128_000),
            max_output_tokens: Some(16_384),
            supports_tools: true,
            supports_vision: true,
            supports_extended_thinking: false,
        },
        ModelInfo {
            id: "o3".into(),
            display_name: "o3".into(),
            provider: "openai".into(),
            context_window: Some(200_000),
            max_output_tokens: Some(100_000),
            supports_tools: true,
            supports_vision: true,
            supports_extended_thinking: true,
        },
        ModelInfo {
            id: "o3-mini".into(),
            display_name: "o3-mini".into(),
            provider: "openai".into(),
            context_window: Some(200_000),
            max_output_tokens: Some(100_000),
            supports_tools: true,
            supports_vision: false,
            supports_extended_thinking: true,
        },
    ]
}

/// Resolve a model name to its provider.
pub fn resolve_model_provider(model: &str) -> Option<&str> {
    if model.starts_with("claude") {
        Some("anthropic")
    } else if model.starts_with("anthropic.") {
        Some("bedrock")
    } else if model.starts_with("gpt-") || model.starts_with("o3") || model.starts_with("o1") {
        Some("openai")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_new_is_empty() {
        let reg = ProviderRegistry::new();
        assert!(reg.available().is_empty());
        assert!(reg.default_name().is_none());
    }

    #[test]
    fn test_register_and_get() {
        let mut reg = ProviderRegistry::new();
        reg.register(ProviderConfig {
            name: "test".into(),
            provider_type: ProviderType::OpenAI,
            base_url: None,
            env_key: None,
            api_key: Some("sk-test".into()),
            aws_region: None,
            max_retries: None,
            headers: None,
        });
        assert_eq!(reg.available().len(), 1);
        assert!(reg.get_config("test").is_some());
        assert!(reg.get_config("other").is_none());
    }

    #[test]
    fn test_set_default() {
        let mut reg = ProviderRegistry::new();
        reg.register(ProviderConfig {
            name: "anthropic".into(),
            provider_type: ProviderType::Anthropic,
            base_url: None,
            env_key: None,
            api_key: Some("key".into()),
            aws_region: None,
            max_retries: None,
            headers: None,
        });
        reg.set_default("anthropic");
        assert_eq!(reg.default_name(), Some("anthropic"));
    }

    #[test]
    fn test_create_anthropic() {
        let mut reg = ProviderRegistry::new();
        reg.register(ProviderConfig {
            name: "anthropic".into(),
            provider_type: ProviderType::Anthropic,
            base_url: None,
            env_key: None,
            api_key: Some("test-key".into()),
            aws_region: None,
            max_retries: None,
            headers: None,
        });
        let provider = reg.create("anthropic");
        assert!(provider.is_ok());
    }

    #[test]
    fn test_create_openai() {
        let mut reg = ProviderRegistry::new();
        reg.register(ProviderConfig {
            name: "openai".into(),
            provider_type: ProviderType::OpenAI,
            base_url: None,
            env_key: None,
            api_key: Some("sk-test".into()),
            aws_region: None,
            max_retries: None,
            headers: None,
        });
        let provider = reg.create("openai");
        assert!(provider.is_ok());
    }

    #[test]
    fn test_create_unknown_fails() {
        let reg = ProviderRegistry::new();
        assert!(reg.create("nonexistent").is_err());
    }

    #[test]
    fn test_create_no_default_fails() {
        let reg = ProviderRegistry::new();
        assert!(reg.create_default().is_err());
    }

    #[test]
    fn test_create_anthropic_no_key_fails() {
        let mut reg = ProviderRegistry::new();
        reg.register(ProviderConfig {
            name: "anthropic".into(),
            provider_type: ProviderType::Anthropic,
            base_url: None,
            env_key: None,
            api_key: None,
            aws_region: None,
            max_retries: None,
            headers: None,
        });
        assert!(reg.create("anthropic").is_err());
    }

    #[test]
    fn test_is_azure_endpoint() {
        assert!(is_azure_endpoint("https://myorg.openai.azure.com/openai"));
        assert!(is_azure_endpoint("https://myorg.cognitiveservices.azure.com"));
        assert!(is_azure_endpoint("https://myorg.aoai.azure.net"));
        assert!(!is_azure_endpoint("https://api.openai.com/v1"));
        assert!(!is_azure_endpoint("https://api.anthropic.com"));
    }

    #[test]
    fn test_resolve_model_provider() {
        assert_eq!(resolve_model_provider("claude-opus-4-6"), Some("anthropic"));
        assert_eq!(resolve_model_provider("claude-sonnet-4-6"), Some("anthropic"));
        assert_eq!(
            resolve_model_provider("anthropic.claude-sonnet-4-6-v1:0"),
            Some("bedrock")
        );
        assert_eq!(resolve_model_provider("gpt-4o"), Some("openai"));
        assert_eq!(resolve_model_provider("o3"), Some("openai"));
        assert_eq!(resolve_model_provider("o3-mini"), Some("openai"));
        assert_eq!(resolve_model_provider("custom-model"), None);
    }

    #[test]
    fn test_builtin_models() {
        let models = builtin_models();
        assert!(models.len() >= 8);

        let anthropic: Vec<_> = models.iter().filter(|m| m.provider == "anthropic").collect();
        assert!(anthropic.len() >= 3);

        let openai: Vec<_> = models.iter().filter(|m| m.provider == "openai").collect();
        assert!(openai.len() >= 3);

        let bedrock: Vec<_> = models.iter().filter(|m| m.provider == "bedrock").collect();
        assert!(bedrock.len() >= 2);
    }

    #[test]
    fn test_model_info_serialization() {
        let model = &builtin_models()[0];
        let json = serde_json::to_string(model).unwrap();
        let parsed: ModelInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.id, model.id);
        assert_eq!(parsed.display_name, model.display_name);
    }

    #[test]
    fn test_provider_config_serialization() {
        let config = ProviderConfig {
            name: "test".into(),
            provider_type: ProviderType::OpenAI,
            base_url: Some("https://api.openai.com/v1".into()),
            env_key: Some("OPENAI_API_KEY".into()),
            api_key: None,
            aws_region: None,
            max_retries: Some(3),
            headers: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: ProviderConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.provider_type, ProviderType::OpenAI);
    }

    #[test]
    fn test_default_registry() {
        let reg = ProviderRegistry::default();
        assert!(reg.available().is_empty());
    }
}
