//! Provider factory abstraction for server-side provider creation.
//!
//! The server needs to create LLM providers for each job execution. Unlike
//! the CLI (which creates one provider per session), the server may run
//! multiple concurrent jobs with different models.
//!
//! `ProviderFactory` wraps the existing `ProviderRegistry` from the providers
//! crate, exposing a simple `create(model)` method suitable for server use.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use cisco_code_api::Provider;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Factory for creating LLM provider instances.
///
/// The server holds one factory and calls `create()` for each job.
/// Implementations can cache/share connections (e.g., reuse HTTP clients).
#[async_trait]
pub trait ProviderFactory: Send + Sync {
    /// Create a provider instance for the given model identifier.
    ///
    /// `model` follows the `[provider/]model-name` convention:
    /// - `"claude-sonnet-4-6"` → resolved via auto-discovery
    /// - `"bedrock/us.anthropic.claude-sonnet-4-6"` → explicit Bedrock
    /// - `"openai/gpt-4.1"` → explicit OpenAI
    async fn create(&self, model: &str) -> Result<Box<dyn Provider>>;
}

// ---------------------------------------------------------------------------
// Default implementation (wraps providers crate ProviderRegistry)
// ---------------------------------------------------------------------------

/// Default factory that delegates to `cisco_code_providers::ProviderRegistry`.
///
/// Created once at server startup and shared across all jobs via `Arc`.
pub struct DefaultProviderFactory {
    registry: cisco_code_providers::ProviderRegistry,
}

impl DefaultProviderFactory {
    /// Create a factory by auto-discovering providers from environment.
    pub fn auto_discover() -> Result<Self> {
        let model_config = cisco_code_providers::ModelConfig::default();
        let registry = cisco_code_providers::ProviderRegistry::auto_discover(model_config)?;
        Ok(Self { registry })
    }
}

#[async_trait]
impl ProviderFactory for DefaultProviderFactory {
    async fn create(&self, model: &str) -> Result<Box<dyn Provider>> {
        let spec = cisco_code_providers::ModelSpec::parse(model);
        if self.registry.has_provider(&spec.provider) {
            let (provider, _resolved_model) = self.registry.provider_for_spec(&spec)?;
            Ok(provider)
        } else {
            // Fallback: try first available provider
            let available = self.registry.available_providers();
            if available.is_empty() {
                anyhow::bail!("No LLM providers available");
            }
            let first = available[0];
            let spec = cisco_code_providers::ModelSpec::new(first, model);
            let (provider, _resolved_model) = self.registry.provider_for_spec(&spec)?;
            Ok(provider)
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// No-op provider factory for tests.
    pub struct NoopProviderFactory;

    #[async_trait]
    impl ProviderFactory for NoopProviderFactory {
        async fn create(&self, _model: &str) -> Result<Box<dyn Provider>> {
            anyhow::bail!("NoopProviderFactory: not a real provider")
        }
    }

    #[test]
    fn default_factory_requires_env() {
        // Without provider env vars, auto_discover should fail gracefully
        // (or succeed if env vars happen to be set in CI).
        let result = DefaultProviderFactory::auto_discover();
        // Just verify it doesn't panic — result depends on environment.
        let _ = result;
    }
}
