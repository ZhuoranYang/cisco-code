//! Provider registry — discovers and manages LLM backends.
//!
//! Providers are lazily initialized based on available API keys.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use cisco_code_api::Provider;

/// Registry of available LLM providers.
pub struct ProviderRegistry {
    providers: HashMap<String, Arc<dyn Provider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    pub fn register(&mut self, name: impl Into<String>, provider: Arc<dyn Provider>) {
        self.providers.insert(name.into(), provider);
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Provider>> {
        self.providers.get(name)
    }

    pub fn available(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }

    /// Auto-discover providers from environment variables.
    pub fn auto_discover() -> Result<Self> {
        let mut registry = Self::new();

        if std::env::var("ANTHROPIC_API_KEY").is_ok() {
            // TODO: register AnthropicProvider
            tracing::info!("Anthropic provider available");
        }

        if std::env::var("OPENAI_API_KEY").is_ok() {
            // TODO: register OpenAIProvider
            tracing::info!("OpenAI provider available");
        }

        if std::env::var("CISCO_AI_API_KEY").is_ok() {
            tracing::info!("Cisco AI provider available");
        }

        let _ = &registry; // suppress unused warning during scaffolding
        Ok(registry)
    }

    /// Resolve a model spec like "anthropic/claude-sonnet-4-6" → (provider, model).
    pub fn resolve_model(spec: &str) -> (String, String) {
        // Handle aliases
        let aliases: HashMap<&str, (&str, &str)> = HashMap::from([
            ("opus", ("anthropic", "claude-opus-4-6")),
            ("sonnet", ("anthropic", "claude-sonnet-4-6")),
            ("haiku", ("anthropic", "claude-haiku-4-5-20251001")),
            ("gpt-5", ("openai", "gpt-5")),
            ("gpt-4o", ("openai", "gpt-4o")),
        ]);

        if let Some((provider, model)) = aliases.get(spec) {
            return (provider.to_string(), model.to_string());
        }

        if let Some((provider, model)) = spec.split_once('/') {
            return (provider.to_string(), model.to_string());
        }

        // Default to anthropic
        ("anthropic".to_string(), spec.to_string())
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}
