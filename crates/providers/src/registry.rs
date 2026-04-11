//! Provider registry — discovers and manages LLM backends.
//!
//! Auto-discovers available providers from environment variables and OAuth tokens:
//! - Bedrock: AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY
//! - OpenAI: OPENAI_API_KEY or stored OAuth tokens (Codex device code flow)
//! - Anthropic: ANTHROPIC_API_KEY

use std::sync::Arc;

use anyhow::Result;
use cisco_code_api::bedrock::BedrockClient;
use cisco_code_api::oauth::CodexAuth;
use cisco_code_api::openai::OpenAIClient;
use cisco_code_api::{AnthropicClient, Provider};

use crate::{ModelClass, ModelConfig, ModelSpec};

/// Registry of discovered provider instances.
pub struct ProviderRegistry {
    bedrock: Option<Arc<BedrockClient>>,
    openai: Option<Arc<OpenAIClient>>,
    anthropic: Option<Arc<AnthropicClient>>,
    /// Local OpenAI-compatible endpoint (Ollama, vLLM, SGLang, LM Studio, etc.)
    local: Option<Arc<OpenAIClient>>,
    pub model_config: ModelConfig,
}

impl ProviderRegistry {
    /// Auto-discover available providers from environment variables and OAuth tokens.
    pub fn auto_discover(model_config: ModelConfig) -> Result<Self> {
        let bedrock = BedrockClient::from_env().ok().map(Arc::new);

        // OpenAI: prefer API key, fall back to OAuth tokens (Codex)
        let openai = if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            let mut client = OpenAIClient::new(key);
            if let Ok(url) = std::env::var("OPENAI_BASE_URL") {
                client = client.with_base_url(url);
            }
            tracing::info!("OpenAI provider: API key mode");
            Some(Arc::new(client))
        } else {
            let codex_auth = Arc::new(CodexAuth::new());
            if codex_auth.has_tokens() {
                tracing::info!("OpenAI provider: OAuth/Codex mode");
                Some(Arc::new(OpenAIClient::with_oauth(codex_auth)))
            } else {
                None
            }
        };

        let anthropic = std::env::var("ANTHROPIC_API_KEY").ok().map(|key| {
            let mut client = AnthropicClient::new(key);
            if let Ok(url) = std::env::var("ANTHROPIC_BASE_URL") {
                client = client.with_base_url(url);
            }
            Arc::new(client)
        });

        // Local: OpenAI-compatible endpoint (Ollama, vLLM, SGLang, LM Studio)
        let local = std::env::var("CISCO_CODE_LOCAL_URL").ok().map(|url| {
            let api_key = std::env::var("CISCO_CODE_LOCAL_API_KEY")
                .unwrap_or_else(|_| "not-needed".into());
            let client = OpenAIClient::new(api_key).with_base_url(url.clone());
            tracing::info!("Local provider: {url}");
            Arc::new(client)
        });

        if bedrock.is_none() && openai.is_none() && anthropic.is_none() && local.is_none() {
            anyhow::bail!(
                "No LLM provider credentials found. Set one of:\n\
                 - AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY (for Bedrock)\n\
                 - OPENAI_API_KEY (for OpenAI / Cisco OAuth)\n\
                 - Run `cisco-code login` (for OpenAI Codex OAuth)\n\
                 - ANTHROPIC_API_KEY (for Anthropic direct)\n\
                 - CISCO_CODE_LOCAL_URL (for local models: Ollama, vLLM, etc.)"
            );
        }

        let discovered: Vec<&str> = [
            bedrock.as_ref().map(|_| "bedrock"),
            openai.as_ref().map(|_| "openai"),
            anthropic.as_ref().map(|_| "anthropic"),
            local.as_ref().map(|_| "local"),
        ]
        .into_iter()
        .flatten()
        .collect();
        tracing::info!("Discovered providers: {}", discovered.join(", "));

        Ok(Self {
            bedrock,
            openai,
            anthropic,
            local,
            model_config,
        })
    }

    /// Get a boxed Provider + model ID for a given model class.
    pub fn provider_for_class(&self, class: ModelClass) -> Result<(Box<dyn Provider>, String)> {
        let spec = self.model_config.resolve(class);
        self.provider_for_spec(spec)
    }

    /// Get a boxed Provider + model ID for a specific ModelSpec.
    pub fn provider_for_spec(&self, spec: &ModelSpec) -> Result<(Box<dyn Provider>, String)> {
        let provider: Box<dyn Provider> = match spec.provider.as_str() {
            "bedrock" => {
                let p = self
                    .bedrock
                    .as_ref()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Bedrock not available (set AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY)"
                        )
                    })?
                    .clone();
                Box::new(p)
            }
            "openai" => {
                let p = self
                    .openai
                    .as_ref()
                    .ok_or_else(|| {
                        anyhow::anyhow!("OpenAI not available (set OPENAI_API_KEY or run `cisco-code login`)")
                    })?
                    .clone();
                Box::new(p)
            }
            "anthropic" => {
                let p = self
                    .anthropic
                    .as_ref()
                    .ok_or_else(|| {
                        anyhow::anyhow!("Anthropic not available (set ANTHROPIC_API_KEY)")
                    })?
                    .clone();
                Box::new(p)
            }
            "local" => {
                let p = self
                    .local
                    .as_ref()
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "Local provider not available (set CISCO_CODE_LOCAL_URL=http://localhost:11434/v1)"
                        )
                    })?
                    .clone();
                Box::new(p)
            }
            other => anyhow::bail!("Unknown provider: {other}"),
        };
        Ok((provider, spec.model.clone()))
    }

    /// List names of available providers.
    pub fn available_providers(&self) -> Vec<&str> {
        let mut providers = Vec::new();
        if self.bedrock.is_some() {
            providers.push("bedrock");
        }
        if self.openai.is_some() {
            providers.push("openai");
        }
        if self.anthropic.is_some() {
            providers.push("anthropic");
        }
        if self.local.is_some() {
            providers.push("local");
        }
        providers
    }

    /// Check if a specific provider is available.
    pub fn has_provider(&self, name: &str) -> bool {
        match name {
            "bedrock" => self.bedrock.is_some(),
            "openai" => self.openai.is_some(),
            "anthropic" => self.anthropic.is_some(),
            "local" => self.local.is_some(),
            _ => false,
        }
    }
}

// Note: `impl<T: Provider> Provider for Arc<T>` is in cisco_code_api::client
// (must be there due to orphan rules — both Provider and Arc are foreign here).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_available_providers_empty_env() {
        // Clear env vars to ensure no providers are found
        // (This test may find providers if env vars happen to be set,
        // but we test the structure regardless)
        let config = ModelConfig::default();
        let result = ProviderRegistry::auto_discover(config);
        // Result depends on env — just verify it returns Ok or the expected error
        if let Err(e) = &result {
            assert!(e.to_string().contains("No LLM provider credentials found"));
        }
    }

    #[test]
    fn test_has_provider_unknown() {
        // Create a registry with no providers (for testing structure)
        let registry = ProviderRegistry {
            bedrock: None,
            openai: None,
            anthropic: None,
            local: None,
            model_config: ModelConfig::default(),
        };
        assert!(!registry.has_provider("bedrock"));
        assert!(!registry.has_provider("openai"));
        assert!(!registry.has_provider("anthropic"));
        assert!(!registry.has_provider("unknown"));
    }

    #[test]
    fn test_available_providers_list() {
        let registry = ProviderRegistry {
            bedrock: None,
            openai: None,
            anthropic: None,
            local: None,
            model_config: ModelConfig::default(),
        };
        assert!(registry.available_providers().is_empty());
    }

    #[test]
    fn test_provider_for_spec_missing_bedrock() {
        let registry = ProviderRegistry {
            bedrock: None,
            openai: None,
            anthropic: None,
            local: None,
            model_config: ModelConfig::default(),
        };
        let spec = ModelSpec::new("bedrock", "some-model");
        let result = registry.provider_for_spec(&spec);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Bedrock not available"));
    }

    #[test]
    fn test_provider_for_spec_unknown_provider() {
        let registry = ProviderRegistry {
            bedrock: None,
            openai: None,
            anthropic: None,
            local: None,
            model_config: ModelConfig::default(),
        };
        let spec = ModelSpec::new("google", "gemini-pro");
        let result = registry.provider_for_spec(&spec);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown provider"));
    }
}
