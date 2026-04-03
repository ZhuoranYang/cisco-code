//! Anthropic (Claude) provider — pure Rust HTTP implementation.
//!
//! Uses reqwest for HTTP and custom SSE parser for streaming.
//! No Python `anthropic` SDK needed.

use anyhow::Result;

/// Anthropic API client.
pub struct AnthropicProvider {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://api.anthropic.com".to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(mut self, url: String) -> Self {
        self.base_url = url;
        self
    }

    /// Build a Messages API request.
    ///
    /// Endpoint: POST /v1/messages
    /// Headers: x-api-key, anthropic-version, content-type
    /// Body: { model, max_tokens, system, messages, tools, stream: true }
    pub async fn create_message_stream(
        &self,
        request: &serde_json::Value,
    ) -> Result<reqwest::Response> {
        let response = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error {status}: {body}");
        }

        Ok(response)
    }
}
