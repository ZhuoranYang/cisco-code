//! AWS Bedrock client for Claude models.
//!
//! Uses SigV4 authentication with the InvokeModel API.
//! Pure Rust implementation — no AWS SDK dependency.
//! Auth: AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY (+ optional AWS_SESSION_TOKEN).

use anyhow::Result;
use std::pin::Pin;

use crate::client::{AssistantEvent, CompletionRequest, Provider};

use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

/// AWS Bedrock client for Claude models.
///
/// Uses InvokeModel API with SigV4 request signing.
/// Supports Claude models via Bedrock's Anthropic integration.
pub struct BedrockClient {
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
    region: String,
    http: reqwest::Client,
    max_retries: u32,
}

impl BedrockClient {
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        region: impl Into<String>,
    ) -> Self {
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: secret_access_key.into(),
            session_token: None,
            region: region.into(),
            http: reqwest::Client::new(),
            max_retries: 2,
        }
    }

    pub fn with_session_token(mut self, token: impl Into<String>) -> Self {
        self.session_token = Some(token.into());
        self
    }

    /// Create from environment variables.
    pub fn from_env() -> Result<Self> {
        let access_key = std::env::var("AWS_ACCESS_KEY_ID")
            .map_err(|_| anyhow::anyhow!("AWS_ACCESS_KEY_ID not set"))?;
        let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY")
            .map_err(|_| anyhow::anyhow!("AWS_SECRET_ACCESS_KEY not set"))?;
        let region = std::env::var("AWS_REGION")
            .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
            .unwrap_or_else(|_| "us-east-1".to_string());

        let mut client = Self::new(access_key, secret_key, region);

        if let Ok(token) = std::env::var("AWS_SESSION_TOKEN") {
            client = client.with_session_token(token);
        }

        Ok(client)
    }

    fn host(&self) -> String {
        format!("bedrock-runtime.{}.amazonaws.com", self.region)
    }

    fn endpoint(&self) -> String {
        format!("https://{}", self.host())
    }

    /// URL-encode a model ID for the Bedrock path.
    fn encode_model_id(model: &str) -> String {
        model.replace(':', "%3A")
    }

    /// Build the request body in Anthropic Messages format (Bedrock variant).
    fn build_body(&self, req: &CompletionRequest) -> serde_json::Value {
        let messages: Vec<serde_json::Value> = req
            .messages
            .iter()
            .map(|msg| {
                serde_json::json!({
                    "role": msg.role,
                    "content": msg.content,
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "anthropic_version": "bedrock-2023-05-31",
            "max_tokens": req.max_tokens,
            "system": req.system_prompt,
            "messages": messages,
        });

        if let Some(temp) = req.temperature {
            body["temperature"] = serde_json::json!(temp);
        }

        if !req.tools.is_empty() {
            let tools: Vec<serde_json::Value> = req
                .tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.input_schema,
                    })
                })
                .collect();
            body["tools"] = serde_json::json!(tools);
        }

        body
    }

    /// Sign and send a request via Bedrock InvokeModel.
    async fn invoke_model(&self, req: &CompletionRequest) -> Result<Vec<AssistantEvent>> {
        let body = self.build_body(req);
        let payload = serde_json::to_vec(&body)?;

        // Use raw model ID in the HTTP URL (reqwest sends it as-is).
        // For SigV4 signing, URI-encode the path (colon → %3A) per AWS spec.
        // Using the same encoded path for both causes double-encoding: reqwest
        // sends `%3A`, and AWS re-encodes `%` → `%25`, producing `%253A`.
        let raw_uri = format!("/model/{}/invoke", req.model);
        let canonical_uri = format!("/model/{}/invoke", Self::encode_model_id(&req.model));

        let now = Utc::now();
        let host = self.host();
        let date_stamp = now.format("%Y%m%d").to_string();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();

        // Build canonical headers (must be sorted by name)
        let session_token_val = self.session_token.clone().unwrap_or_default();
        let mut header_pairs: Vec<(&str, &str)> = vec![
            ("content-type", "application/json"),
            ("host", &host),
            ("x-amz-date", &amz_date),
        ];
        if self.session_token.is_some() {
            header_pairs.push(("x-amz-security-token", &session_token_val));
        }
        header_pairs.sort_by_key(|h| h.0);

        let canonical_headers: String = header_pairs
            .iter()
            .map(|(k, v)| format!("{k}:{v}\n"))
            .collect();

        let signed_headers: String = header_pairs
            .iter()
            .map(|(k, _)| *k)
            .collect::<Vec<_>>()
            .join(";");

        let payload_hash = sha256_hex(&payload);

        // Canonical request (use encoded URI for signing)
        let canonical_request = format!(
            "POST\n{canonical_uri}\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );

        // String to sign
        let scope = format!("{date_stamp}/{}/bedrock/aws4_request", self.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
            sha256_hex(canonical_request.as_bytes())
        );

        // Derive signing key and compute signature
        let signing_key = derive_signing_key(
            &self.secret_access_key,
            &date_stamp,
            &self.region,
            "bedrock",
        );
        let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes()));

        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
            self.access_key_id
        );

        // Send request (use raw URI to avoid double-encoding)
        let mut request_builder = self
            .http
            .post(format!("{}{raw_uri}", self.endpoint()))
            .header("content-type", "application/json")
            .header("x-amz-date", &amz_date)
            .header("authorization", &authorization);

        if let Some(ref token) = self.session_token {
            request_builder = request_builder.header("x-amz-security-token", token);
        }

        let resp = request_builder.body(payload).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Bedrock API error {status}: {text}");
        }

        let response_body: serde_json::Value = resp.json().await?;
        self.parse_response(response_body)
    }

    /// Parse Bedrock/Anthropic response JSON into AssistantEvents.
    fn parse_response(&self, body: serde_json::Value) -> Result<Vec<AssistantEvent>> {
        let mut events = Vec::new();

        // Usage
        if let Some(usage) = body.get("usage") {
            events.push(AssistantEvent::Usage {
                input_tokens: usage["input_tokens"].as_u64().unwrap_or(0),
                output_tokens: usage["completion_tokens"]
                    .as_u64()
                    .or(usage["output_tokens"].as_u64())
                    .unwrap_or(0),
            });
        }

        // Content blocks
        if let Some(content) = body["content"].as_array() {
            for block in content {
                match block["type"].as_str() {
                    Some("text") => {
                        if let Some(text) = block["text"].as_str() {
                            events.push(AssistantEvent::TextDelta(text.to_string()));
                        }
                    }
                    Some("tool_use") => {
                        let id = block["id"].as_str().unwrap_or("").to_string();
                        let name = block["name"].as_str().unwrap_or("").to_string();
                        let input = block["input"].clone();
                        events.push(AssistantEvent::ToolUse { id, name, input });
                    }
                    _ => {}
                }
            }
        }

        // Stop reason
        if let Some(reason) = body["stop_reason"].as_str() {
            events.push(AssistantEvent::MessageStop {
                stop_reason: reason.to_string(),
            });
        }

        Ok(events)
    }

    pub async fn invoke_with_retry(&self, req: &CompletionRequest) -> Result<Vec<AssistantEvent>> {
        let mut last_err = None;
        for attempt in 0..=self.max_retries {
            match self.invoke_model(req).await {
                Ok(ev) => return Ok(ev),
                Err(e) => {
                    let msg = e.to_string();
                    let retryable =
                        msg.contains("429") || msg.contains("500") || msg.contains("503");
                    if retryable && attempt < self.max_retries {
                        let backoff = std::time::Duration::from_millis(500 * 2u64.pow(attempt));
                        tracing::warn!(
                            "Bedrock error (attempt {}), retry in {backoff:?}: {msg}",
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

impl Provider for BedrockClient {
    fn stream(
        &self,
        request: CompletionRequest,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<AssistantEvent>>> + Send + '_>> {
        Box::pin(async move { self.invoke_with_retry(&request).await })
    }
}

// ---------------------------------------------------------------------------
// SigV4 helpers
// ---------------------------------------------------------------------------

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn derive_signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bedrock_client_creation() {
        let client = BedrockClient::new("AKID", "SECRET", "us-west-2");
        assert_eq!(client.access_key_id, "AKID");
        assert_eq!(client.secret_access_key, "SECRET");
        assert_eq!(client.region, "us-west-2");
        assert!(client.session_token.is_none());
        assert_eq!(client.max_retries, 2);
    }

    #[test]
    fn test_bedrock_with_session_token() {
        let client =
            BedrockClient::new("AKID", "SECRET", "us-east-1").with_session_token("TOKEN123");
        assert_eq!(client.session_token.as_deref(), Some("TOKEN123"));
    }

    #[test]
    fn test_encode_model_id() {
        assert_eq!(
            BedrockClient::encode_model_id("anthropic.claude-3-5-sonnet-20241022-v2:0"),
            "anthropic.claude-3-5-sonnet-20241022-v2%3A0"
        );
        assert_eq!(
            BedrockClient::encode_model_id("anthropic.claude-3-haiku-20240307-v1:0"),
            "anthropic.claude-3-haiku-20240307-v1%3A0"
        );
        // No colon — no encoding needed
        assert_eq!(
            BedrockClient::encode_model_id("my-model"),
            "my-model"
        );
    }

    #[test]
    fn test_bedrock_host() {
        let client = BedrockClient::new("AKID", "SECRET", "us-west-2");
        assert_eq!(client.host(), "bedrock-runtime.us-west-2.amazonaws.com");
    }

    #[test]
    fn test_bedrock_endpoint() {
        let client = BedrockClient::new("AKID", "SECRET", "eu-west-1");
        assert_eq!(
            client.endpoint(),
            "https://bedrock-runtime.eu-west-1.amazonaws.com"
        );
    }

    #[test]
    fn test_sha256_hex_empty() {
        let hash = sha256_hex(b"");
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sha256_hex_known_value() {
        let hash = sha256_hex(b"hello");
        assert_eq!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_derive_signing_key_length() {
        let key = derive_signing_key(
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20130524",
            "us-east-1",
            "s3",
        );
        assert_eq!(key.len(), 32); // HMAC-SHA256 produces 32 bytes
    }

    #[test]
    fn test_hmac_sha256_deterministic() {
        let a = hmac_sha256(b"key", b"data");
        let b = hmac_sha256(b"key", b"data");
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn test_bedrock_build_body_basic() {
        let client = BedrockClient::new("AKID", "SECRET", "us-east-1");
        let req = CompletionRequest {
            model: "anthropic.claude-3-5-sonnet-20241022-v2:0".into(),
            system_prompt: "You are helpful.".into(),
            messages: vec![crate::client::ApiMessage {
                role: "user".into(),
                content: serde_json::json!("hello"),
            }],
            tools: vec![],
            max_tokens: 4096,
            temperature: None,
            thinking: None,
            system_blocks: None,
        };

        let body = client.build_body(&req);
        assert_eq!(body["anthropic_version"], "bedrock-2023-05-31");
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["system"], "You are helpful.");
        assert!(body.get("temperature").is_none());
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn test_bedrock_build_body_with_tools() {
        let client = BedrockClient::new("AKID", "SECRET", "us-east-1");
        let req = CompletionRequest {
            model: "anthropic.claude-3-5-sonnet-20241022-v2:0".into(),
            system_prompt: "Agent".into(),
            messages: vec![],
            tools: vec![cisco_code_protocol::ToolDefinition {
                name: "Bash".into(),
                description: "Run commands".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }],
            max_tokens: 2048,
            temperature: Some(0.5),
            thinking: None,
            system_blocks: None,
        };

        let body = client.build_body(&req);
        assert!(body["tools"].is_array());
        assert_eq!(body["tools"][0]["name"], "Bash");
        assert_eq!(body["temperature"], 0.5);
    }

    #[test]
    fn test_bedrock_parse_response_text() {
        let client = BedrockClient::new("AKID", "SECRET", "us-east-1");
        let response = serde_json::json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "text", "text": "Hello!"}
            ],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });

        let events = client.parse_response(response).unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantEvent::TextDelta(t) if t == "Hello!")));
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantEvent::MessageStop { stop_reason } if stop_reason == "end_turn")));
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantEvent::Usage { input_tokens: 10, output_tokens: 5 })));
    }

    #[test]
    fn test_bedrock_parse_response_tool_use() {
        let client = BedrockClient::new("AKID", "SECRET", "us-east-1");
        let response = serde_json::json!({
            "content": [
                {
                    "type": "tool_use",
                    "id": "tu_1",
                    "name": "Read",
                    "input": {"file_path": "/tmp/test.txt"}
                }
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 20, "output_tokens": 15}
        });

        let events = client.parse_response(response).unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantEvent::ToolUse { name, .. } if name == "Read")));
        assert!(events
            .iter()
            .any(|e| matches!(e, AssistantEvent::MessageStop { stop_reason } if stop_reason == "tool_use")));
    }

    #[test]
    fn test_bedrock_parse_response_mixed_content() {
        let client = BedrockClient::new("AKID", "SECRET", "us-east-1");
        let response = serde_json::json!({
            "content": [
                {"type": "text", "text": "I'll read the file."},
                {
                    "type": "tool_use",
                    "id": "tu_2",
                    "name": "Read",
                    "input": {"file_path": "/etc/hosts"}
                }
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 30, "output_tokens": 20}
        });

        let events = client.parse_response(response).unwrap();
        let text_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AssistantEvent::TextDelta(_)))
            .collect();
        let tool_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AssistantEvent::ToolUse { .. }))
            .collect();
        assert_eq!(text_events.len(), 1);
        assert_eq!(tool_events.len(), 1);
    }

    #[test]
    fn test_bedrock_parse_empty_response() {
        let client = BedrockClient::new("AKID", "SECRET", "us-east-1");
        let response = serde_json::json!({});

        let events = client.parse_response(response).unwrap();
        assert!(events.is_empty());
    }
}
