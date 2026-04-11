//! OpenAI Chat Completions API client.
//!
//! Supports:
//! - Standard OpenAI API (api.openai.com) with API key
//! - Cisco's OpenAI OAuth proxy with Bearer token
//! - OpenAI Codex OAuth (ChatGPT subscription via device code flow)
//! - Azure OpenAI and any OpenAI-compatible endpoint
//!
//! Auth modes:
//! - API key: Bearer token from OPENAI_API_KEY
//! - OAuth: Auto-refreshing token from CodexAuth (device code flow)

use anyhow::Result;
use std::pin::Pin;
use std::sync::Arc;

use crate::client::{AssistantEvent, CompletionRequest, Provider};
use crate::oauth::{CodexAuth, CODEX_API_ENDPOINT};
use crate::sse::SseParser;

/// OpenAI-compatible client with API key or OAuth support.
///
/// Works with:
/// - OpenAI API (api.openai.com)
/// - OpenAI Codex (chatgpt.com/backend-api via OAuth)
/// - Cisco OpenAI OAuth proxy
/// - Azure OpenAI
/// - Any OpenAI-compatible endpoint (Groq, Together, etc.)
pub struct OpenAIClient {
    api_key: String,
    oauth: Option<Arc<CodexAuth>>,
    base_url: String,
    http: reqwest::Client,
    max_retries: u32,
}

impl OpenAIClient {
    /// Create with an API key (standard mode).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            oauth: None,
            base_url: "https://api.openai.com/v1".to_string(),
            http: reqwest::Client::new(),
            max_retries: 2,
        }
    }

    /// Create with OAuth (Codex mode).
    ///
    /// Uses the Codex API endpoint and auto-refreshing OAuth tokens.
    pub fn with_oauth(codex_auth: Arc<CodexAuth>) -> Self {
        Self {
            api_key: String::new(),
            oauth: Some(codex_auth),
            base_url: CODEX_API_ENDPOINT.to_string(),
            http: reqwest::Client::new(),
            max_retries: 2,
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Whether this client uses OAuth (Codex mode).
    pub fn is_oauth_mode(&self) -> bool {
        self.oauth.is_some()
    }

    /// Resolve authentication for the current request.
    ///
    /// Returns (bearer_token, extra_headers).
    async fn resolve_auth(&self) -> Result<(String, Vec<(String, String)>)> {
        match &self.oauth {
            Some(auth) => {
                let (token, account_id) = auth.get_access_token().await?;
                let mut headers = vec![("originator".into(), "cisco-code".into())];
                if let Some(acct) = account_id {
                    headers.push(("chatgpt-account-id".into(), acct));
                }
                Ok((token, headers))
            }
            None => Ok((self.api_key.clone(), vec![])),
        }
    }

    /// Build the request body for OpenAI Chat Completions.
    fn build_body(&self, req: &CompletionRequest) -> serde_json::Value {
        let mut messages = vec![serde_json::json!({
            "role": "system",
            "content": req.system_prompt,
        })];

        for msg in &req.messages {
            messages.push(serde_json::json!({
                "role": msg.role,
                "content": msg.content,
            }));
        }

        let mut body = serde_json::json!({
            "model": req.model,
            "messages": messages,
            "max_tokens": req.max_tokens,
            "stream": true,
        });

        if let Some(temp) = req.temperature {
            body["temperature"] = serde_json::json!(temp);
        }

        // Convert tools to OpenAI function calling format
        if !req.tools.is_empty() {
            let tools: Vec<serde_json::Value> = req
                .tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.input_schema,
                        }
                    })
                })
                .collect();
            body["tools"] = serde_json::json!(tools);
        }

        body
    }

    async fn stream_raw(&self, req: &CompletionRequest) -> Result<Vec<AssistantEvent>> {
        let body = self.build_body(req);
        let (token, extra_headers) = self.resolve_auth().await?;

        // In OAuth/Codex mode, base_url IS the full endpoint.
        // In API key mode, append /chat/completions.
        let url = if self.is_oauth_mode() {
            self.base_url.clone()
        } else {
            format!("{}/chat/completions", self.base_url)
        };

        let mut request = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .header("content-type", "application/json");

        for (key, value) in &extra_headers {
            request = request.header(key.as_str(), value.as_str());
        }

        let resp = request.json(&body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API error {status}: {text}");
        }

        let bytes = resp.bytes().await?;
        let mut parser = SseParser::new();
        let frames = parser.push(&bytes)?;

        let mut events = Vec::new();
        let mut tool_calls: Vec<OpenAIToolCall> = Vec::new();

        for frame in frames {
            let chunk: serde_json::Value = match serde_json::from_str(&frame.data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if let Some(choices) = chunk["choices"].as_array() {
                for choice in choices {
                    let delta = &choice["delta"];
                    let finish_reason = choice["finish_reason"].as_str();

                    // Text content
                    if let Some(content) = delta["content"].as_str() {
                        if !content.is_empty() {
                            events.push(AssistantEvent::TextDelta(content.to_string()));
                        }
                    }

                    // Tool calls (streamed incrementally)
                    if let Some(tcs) = delta["tool_calls"].as_array() {
                        for tc in tcs {
                            let index = tc["index"].as_u64().unwrap_or(0) as usize;
                            while tool_calls.len() <= index {
                                tool_calls.push(OpenAIToolCall::default());
                            }
                            if let Some(id) = tc["id"].as_str() {
                                tool_calls[index].id = id.to_string();
                            }
                            if let Some(name) = tc["function"]["name"].as_str() {
                                tool_calls[index].name = name.to_string();
                            }
                            if let Some(args) = tc["function"]["arguments"].as_str() {
                                tool_calls[index].arguments.push_str(args);
                            }
                        }
                    }

                    // Finish reason
                    if let Some(reason) = finish_reason {
                        // Emit accumulated tool calls
                        for tc in &tool_calls {
                            if !tc.name.is_empty() {
                                let input: serde_json::Value =
                                    serde_json::from_str(&tc.arguments)
                                        .unwrap_or(serde_json::json!({}));
                                events.push(AssistantEvent::ToolUse {
                                    id: tc.id.clone(),
                                    name: tc.name.clone(),
                                    input,
                                });
                            }
                        }
                        tool_calls.clear();

                        let stop_reason = match reason {
                            "stop" => "end_turn",
                            "tool_calls" => "tool_use",
                            "length" => "max_tokens",
                            other => other,
                        };
                        events.push(AssistantEvent::MessageStop {
                            stop_reason: stop_reason.to_string(),
                        });
                    }
                }
            }

            // Usage
            if let Some(usage) = chunk.get("usage") {
                events.push(AssistantEvent::Usage {
                    input_tokens: usage["prompt_tokens"].as_u64().unwrap_or(0),
                    output_tokens: usage["completion_tokens"].as_u64().unwrap_or(0),
                });
            }
        }

        Ok(events)
    }

    pub async fn stream_with_retry(&self, req: &CompletionRequest) -> Result<Vec<AssistantEvent>> {
        let mut last_err = None;
        for attempt in 0..=self.max_retries {
            match self.stream_raw(req).await {
                Ok(ev) => return Ok(ev),
                Err(e) => {
                    let msg = e.to_string();
                    let retryable =
                        msg.contains("429") || msg.contains("500") || msg.contains("502");
                    if retryable && attempt < self.max_retries {
                        let backoff = std::time::Duration::from_millis(500 * 2u64.pow(attempt));
                        tracing::warn!(
                            "OpenAI error (attempt {}), retry in {backoff:?}: {msg}",
                            attempt + 1
                        );
                        tokio::time::sleep(backoff).await;
                        last_err = Some(e);
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("retries exhausted")))
    }
}

impl Provider for OpenAIClient {
    fn stream(
        &self,
        request: CompletionRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<AssistantEvent>>> + Send + '_>> {
        Box::pin(self.stream_with_retry(&request))
    }
}

#[derive(Default)]
struct OpenAIToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_client_creation() {
        let client = OpenAIClient::new("sk-test");
        assert_eq!(client.api_key, "sk-test");
        assert_eq!(client.base_url, "https://api.openai.com/v1");
        assert!(!client.is_oauth_mode());
    }

    #[test]
    fn test_openai_custom_base_url() {
        let client =
            OpenAIClient::new("token").with_base_url("https://cisco-openai-proxy.example.com/v1");
        assert_eq!(
            client.base_url,
            "https://cisco-openai-proxy.example.com/v1"
        );
    }

    #[test]
    fn test_openai_oauth_mode() {
        let auth = Arc::new(CodexAuth::new());
        let client = OpenAIClient::with_oauth(auth);
        assert!(client.is_oauth_mode());
        assert_eq!(client.base_url, CODEX_API_ENDPOINT);
        assert!(client.api_key.is_empty());
    }

    #[test]
    fn test_openai_build_body_basic() {
        let client = OpenAIClient::new("key");
        let req = CompletionRequest {
            model: "gpt-4o".into(),
            system_prompt: "You are helpful.".into(),
            messages: vec![crate::client::ApiMessage {
                role: "user".into(),
                content: serde_json::json!("hello"),
            }],
            tools: vec![],
            max_tokens: 2048,
            temperature: None,
            thinking: None,
            system_blocks: None,
        };
        let body = client.build_body(&req);

        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["stream"], true);
        assert_eq!(body["max_tokens"], 2048);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn test_openai_build_body_with_tools() {
        let client = OpenAIClient::new("key");
        let req = CompletionRequest {
            model: "gpt-4o".into(),
            system_prompt: "agent".into(),
            messages: vec![],
            tools: vec![cisco_code_protocol::ToolDefinition {
                name: "Bash".into(),
                description: "Run commands".into(),
                input_schema: serde_json::json!({"type": "object", "properties": {"command": {"type": "string"}}}),
            }],
            max_tokens: 1024,
            temperature: Some(0.5),
            thinking: None,
            system_blocks: None,
        };
        let body = client.build_body(&req);

        assert!(body["tools"].is_array());
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "Bash");
        assert_eq!(body["temperature"], 0.5);
    }

    #[tokio::test]
    async fn test_openai_resolve_auth_api_key() {
        let client = OpenAIClient::new("sk-test-key");
        let (token, headers) = client.resolve_auth().await.unwrap();
        assert_eq!(token, "sk-test-key");
        assert!(headers.is_empty());
    }
}
