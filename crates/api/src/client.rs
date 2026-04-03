//! API client trait and Anthropic streaming implementation.
//!
//! Pure Rust HTTP via reqwest. No Python SDK needed.
//! Pattern from Claw-Code-Parity: reqwest + custom SSE parsing.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::sse::{AnthropicStreamEvent, SseParser};
use cisco_code_protocol::ToolDefinition;

// ---------------------------------------------------------------------------
// Provider trait
// ---------------------------------------------------------------------------

/// LLM backends implement this trait.
pub trait Provider: Send + Sync {
    /// Send a completion request and collect streamed events.
    fn stream(
        &self,
        request: CompletionRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<AssistantEvent>>> + Send + '_>>;
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct CompletionRequest {
    pub model: String,
    pub system_prompt: String,
    pub messages: Vec<ApiMessage>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
}

/// Message format sent to the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMessage {
    pub role: String,
    pub content: serde_json::Value,
}

/// Events parsed from a streaming LLM response.
#[derive(Debug, Clone)]
pub enum AssistantEvent {
    TextDelta(String),
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    MessageStop {
        stop_reason: String,
    },
}

// ---------------------------------------------------------------------------
// Anthropic client
// ---------------------------------------------------------------------------

pub struct AnthropicClient {
    api_key: String,
    base_url: String,
    http: reqwest::Client,
    max_retries: u32,
}

impl AnthropicClient {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.anthropic.com".to_string(),
            http: reqwest::Client::new(),
            max_retries: 2,
        }
    }

    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    fn build_body(&self, req: &CompletionRequest) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": req.model,
            "max_tokens": req.max_tokens,
            "system": req.system_prompt,
            "messages": req.messages,
            "stream": true,
        });
        if !req.tools.is_empty() {
            body["tools"] = serde_json::json!(
                req.tools.iter().map(|t| serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })).collect::<Vec<_>>()
            );
        }
        if let Some(temp) = req.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        body
    }

    async fn stream_raw(&self, req: &CompletionRequest) -> Result<Vec<AssistantEvent>> {
        let body = self.build_body(req);

        let resp = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API error {status}: {text}");
        }

        let bytes = resp.bytes().await?;
        let mut parser = SseParser::new();
        let frames = parser.push(&bytes)?;

        let mut events = Vec::new();
        let mut blocks: Vec<BlockAcc> = Vec::new();

        for frame in frames {
            let sse: AnthropicStreamEvent = serde_json::from_str(&frame.data)?;
            match sse {
                AnthropicStreamEvent::MessageStart { message } => {
                    if let Some(u) = message.get("usage") {
                        events.push(AssistantEvent::Usage {
                            input_tokens: u["input_tokens"].as_u64().unwrap_or(0),
                            output_tokens: u["output_tokens"].as_u64().unwrap_or(0),
                        });
                    }
                }
                AnthropicStreamEvent::ContentBlockStart {
                    index,
                    content_block,
                } => {
                    while blocks.len() <= index {
                        blocks.push(BlockAcc::default());
                    }
                    let bt = content_block["type"].as_str().unwrap_or("");
                    blocks[index].block_type = bt.to_string();
                    if bt == "tool_use" {
                        blocks[index].tool_id =
                            content_block["id"].as_str().unwrap_or("").to_string();
                        blocks[index].tool_name =
                            content_block["name"].as_str().unwrap_or("").to_string();
                    }
                }
                AnthropicStreamEvent::ContentBlockDelta { index, delta } => {
                    if index >= blocks.len() {
                        continue;
                    }
                    match delta["type"].as_str().unwrap_or("") {
                        "text_delta" => {
                            if let Some(t) = delta["text"].as_str() {
                                events.push(AssistantEvent::TextDelta(t.to_string()));
                            }
                        }
                        "input_json_delta" => {
                            if let Some(j) = delta["partial_json"].as_str() {
                                blocks[index].tool_json.push_str(j);
                            }
                        }
                        _ => {}
                    }
                }
                AnthropicStreamEvent::ContentBlockStop { index } => {
                    if index < blocks.len() && blocks[index].block_type == "tool_use" {
                        let b = &blocks[index];
                        let input: serde_json::Value = serde_json::from_str(&b.tool_json)
                            .unwrap_or(serde_json::Value::Object(Default::default()));
                        events.push(AssistantEvent::ToolUse {
                            id: b.tool_id.clone(),
                            name: b.tool_name.clone(),
                            input,
                        });
                    }
                }
                AnthropicStreamEvent::MessageDelta { delta, usage } => {
                    if let Some(r) = delta["stop_reason"].as_str() {
                        events.push(AssistantEvent::MessageStop {
                            stop_reason: r.to_string(),
                        });
                    }
                    if let Some(out) = usage["output_tokens"].as_u64() {
                        events.push(AssistantEvent::Usage {
                            input_tokens: 0,
                            output_tokens: out,
                        });
                    }
                }
                AnthropicStreamEvent::Error { error } => {
                    let msg = error["message"].as_str().unwrap_or("unknown error");
                    anyhow::bail!("Anthropic stream error: {msg}");
                }
                _ => {}
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
                        msg.contains("429") || msg.contains("500") || msg.contains("529");
                    if retryable && attempt < self.max_retries {
                        let backoff =
                            std::time::Duration::from_millis(500 * 2u64.pow(attempt));
                        tracing::warn!("API error (attempt {}), retry in {backoff:?}: {msg}", attempt + 1);
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

impl Provider for AnthropicClient {
    fn stream(
        &self,
        request: CompletionRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<AssistantEvent>>> + Send + '_>> {
        Box::pin(self.stream_with_retry(&request))
    }
}

#[derive(Default)]
struct BlockAcc {
    block_type: String,
    tool_id: String,
    tool_name: String,
    tool_json: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_client_creation() {
        let client = AnthropicClient::new("test-key");
        assert_eq!(client.api_key, "test-key");
        assert_eq!(client.base_url, "https://api.anthropic.com");
        assert_eq!(client.max_retries, 2);
    }

    #[test]
    fn test_custom_base_url() {
        let client = AnthropicClient::new("key").with_base_url("https://custom.api.example.com");
        assert_eq!(client.base_url, "https://custom.api.example.com");
    }

    #[test]
    fn test_build_body_minimal() {
        let client = AnthropicClient::new("key");
        let req = CompletionRequest {
            model: "claude-sonnet-4-6".into(),
            system_prompt: "You are helpful.".into(),
            messages: vec![ApiMessage {
                role: "user".into(),
                content: serde_json::json!("Hello"),
            }],
            tools: vec![],
            max_tokens: 1024,
            temperature: None,
        };
        let body = client.build_body(&req);

        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["system"], "You are helpful.");
        assert_eq!(body["stream"], true);
        assert!(body.get("tools").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn test_build_body_with_tools_and_temperature() {
        let client = AnthropicClient::new("key");
        let req = CompletionRequest {
            model: "claude-opus-4-6".into(),
            system_prompt: "Agent".into(),
            messages: vec![],
            tools: vec![cisco_code_protocol::ToolDefinition {
                name: "Read".into(),
                description: "Read a file".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }],
            max_tokens: 4096,
            temperature: Some(0.7),
        };
        let body = client.build_body(&req);

        assert!(body["tools"].is_array());
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(body["tools"][0]["name"], "Read");
        assert_eq!(body["temperature"], 0.7);
    }

    #[test]
    fn test_completion_request_serialization() {
        let req = CompletionRequest {
            model: "test".into(),
            system_prompt: "sys".into(),
            messages: vec![ApiMessage {
                role: "user".into(),
                content: serde_json::json!("hi"),
            }],
            tools: vec![],
            max_tokens: 100,
            temperature: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["model"], "test");
        assert!(json.get("temperature").is_none()); // skip_serializing_if
    }

    #[test]
    fn test_assistant_event_variants() {
        let text = AssistantEvent::TextDelta("hello".into());
        assert!(matches!(text, AssistantEvent::TextDelta(ref t) if t == "hello"));

        let tool = AssistantEvent::ToolUse {
            id: "tu_1".into(),
            name: "Bash".into(),
            input: serde_json::json!({"command": "ls"}),
        };
        assert!(matches!(tool, AssistantEvent::ToolUse { ref name, .. } if name == "Bash"));

        let stop = AssistantEvent::MessageStop {
            stop_reason: "end_turn".into(),
        };
        assert!(matches!(stop, AssistantEvent::MessageStop { ref stop_reason } if stop_reason == "end_turn"));
    }
}
