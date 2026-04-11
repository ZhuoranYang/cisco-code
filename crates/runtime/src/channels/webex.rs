//! Webex channel — receive messages from Cisco Webex, respond back.
//!
//! Uses Webex webhooks: Webex sends HTTP POST to our webhook URL when
//! someone messages the bot. We fetch the message content, feed it to
//! the agent, and post the response back to the same room/thread.
//!
//! Auth: WEBEX_TOKEN environment variable (Bot Access Token).
//! Setup: Create a Webex bot at https://developer.webex.com, register a
//! webhook pointing to your server's /webhook/webex endpoint.

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};

const WEBEX_API_BASE: &str = "https://webexapis.com/v1";

/// Webex channel configuration.
#[derive(Debug, Clone)]
pub struct WebexChannelConfig {
    /// Webex Bot Access Token.
    pub token: String,
    /// Port to listen for webhooks on.
    pub webhook_port: u16,
    /// Optional: only accept messages from this person's email.
    pub allowed_sender: Option<String>,
}

impl WebexChannelConfig {
    /// Create config from environment variables.
    pub fn from_env() -> anyhow::Result<Self> {
        let token = std::env::var("WEBEX_TOKEN")
            .map_err(|_| anyhow::anyhow!("WEBEX_TOKEN not set"))?;
        let port = std::env::var("WEBEX_WEBHOOK_PORT")
            .unwrap_or_else(|_| "8080".into())
            .parse()
            .unwrap_or(8080);
        let allowed = std::env::var("WEBEX_ALLOWED_SENDER").ok();

        Ok(Self {
            token,
            webhook_port: port,
            allowed_sender: allowed,
        })
    }
}

/// Webex channel that receives webhook events and responds via the Webex API.
pub struct WebexChannel {
    config: WebexChannelConfig,
    http: reqwest::Client,
}

impl WebexChannel {
    pub fn new(config: WebexChannelConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    /// Fetch the full message content from Webex API.
    /// (Webhooks only send the message ID, not the content.)
    #[allow(dead_code)] // will be used when webhook message polling is wired up
    async fn fetch_message(&self, message_id: &str) -> anyhow::Result<serde_json::Value> {
        let resp = self
            .http
            .get(format!("{WEBEX_API_BASE}/messages/{message_id}"))
            .bearer_auth(&self.config.token)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Webex API error {status}: {text}");
        }

        Ok(resp.json().await?)
    }

    /// Post a message to Webex.
    async fn post_message(
        &self,
        room_id: &str,
        text: &str,
        parent_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut body = serde_json::json!({
            "roomId": room_id,
            "markdown": text,
        });

        if let Some(pid) = parent_id {
            body["parentId"] = serde_json::json!(pid);
        }

        let resp = self
            .http
            .post(format!("{WEBEX_API_BASE}/messages"))
            .bearer_auth(&self.config.token)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Webex send error {status}: {text}");
        }

        Ok(())
    }

    /// Get the bot's own identity to filter self-messages.
    async fn get_bot_id(&self) -> anyhow::Result<String> {
        let resp = self
            .http
            .get(format!("{WEBEX_API_BASE}/people/me"))
            .bearer_auth(&self.config.token)
            .send()
            .await?;

        let body: serde_json::Value = resp.json().await?;
        Ok(body["id"].as_str().unwrap_or("").to_string())
    }
}

#[async_trait::async_trait]
impl Channel for WebexChannel {
    fn name(&self) -> &str {
        "webex"
    }

    async fn start(&self) -> anyhow::Result<MessageStream> {
        let (tx, rx) = mpsc::channel(32);
        let token = self.config.token.clone();
        let port = self.config.webhook_port;
        let allowed_sender = self.config.allowed_sender.clone();

        // Get bot's own ID to filter self-messages
        let bot_id = self.get_bot_id().await.unwrap_or_default();

        let http = self.http.clone();

        // Spawn webhook listener
        tokio::spawn(async move {
            // Simple webhook server using a raw TCP listener + manual HTTP parsing
            // In production, you'd use axum/warp, but keeping deps minimal here.
            let listener = match tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("Failed to bind Webex webhook listener on port {port}: {e}");
                    return;
                }
            };

            tracing::info!("Webex webhook listening on port {port}");

            loop {
                let (stream, _addr) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(e) => {
                        tracing::error!("Webex webhook accept error: {e}");
                        continue;
                    }
                };

                let tx = tx.clone();
                let token = token.clone();
                let bot_id = bot_id.clone();
                let allowed_sender = allowed_sender.clone();
                let http = http.clone();

                tokio::spawn(async move {
                    if let Err(e) = handle_webhook_connection(
                        stream,
                        tx,
                        &token,
                        &bot_id,
                        allowed_sender.as_deref(),
                        &http,
                    )
                    .await
                    {
                        tracing::debug!("Webhook connection error: {e}");
                    }
                });
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }

    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> anyhow::Result<()> {
        let room_id = msg.metadata["roomId"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing roomId in message metadata"))?;

        let parent_id = response
            .thread_id
            .as_deref()
            .or_else(|| msg.metadata["parentId"].as_str());

        self.post_message(room_id, &response.content, parent_id)
            .await
    }

    async fn send_status(
        &self,
        _status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> anyhow::Result<()> {
        // Webex doesn't have typing indicators via API, so status is a no-op
        Ok(())
    }

    async fn health_check(&self) -> anyhow::Result<()> {
        let resp = self
            .http
            .get(format!("{WEBEX_API_BASE}/people/me"))
            .bearer_auth(&self.config.token)
            .send()
            .await?;

        if resp.status().is_success() {
            Ok(())
        } else {
            anyhow::bail!("Webex health check failed: {}", resp.status())
        }
    }
}

/// Handle a single webhook HTTP connection.
async fn handle_webhook_connection(
    mut stream: tokio::net::TcpStream,
    tx: mpsc::Sender<IncomingMessage>,
    token: &str,
    bot_id: &str,
    allowed_sender: Option<&str>,
    http: &reqwest::Client,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // Read HTTP request (simplified — production would use hyper/axum)
    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Extract body (after \r\n\r\n)
    let body = request
        .split("\r\n\r\n")
        .nth(1)
        .unwrap_or("");

    // Respond 200 immediately (Webex expects fast response)
    let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
    stream.write_all(response.as_bytes()).await?;

    // Parse webhook payload
    let payload: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return Ok(()), // Not JSON, ignore
    };

    // Only handle "messages" resource with "created" event
    let resource = payload["resource"].as_str().unwrap_or("");
    let event = payload["event"].as_str().unwrap_or("");
    if resource != "messages" || event != "created" {
        return Ok(());
    }

    let message_id = match payload["data"]["id"].as_str() {
        Some(id) => id.to_string(),
        None => return Ok(()),
    };

    let person_id = payload["data"]["personId"]
        .as_str()
        .unwrap_or("");

    // Skip bot's own messages
    if person_id == bot_id {
        return Ok(());
    }

    // Fetch full message content from Webex API
    let msg_resp = http
        .get(format!("{WEBEX_API_BASE}/messages/{message_id}"))
        .bearer_auth(token)
        .send()
        .await?;

    let msg_body: serde_json::Value = msg_resp.json().await?;
    let text = msg_body["text"].as_str().unwrap_or("").to_string();
    let person_email = msg_body["personEmail"].as_str().unwrap_or("").to_string();
    let room_id = msg_body["roomId"].as_str().unwrap_or("").to_string();
    let parent_id = msg_body["parentId"].as_str().map(|s| s.to_string());

    // Check allowed sender
    if let Some(allowed) = allowed_sender {
        if person_email != allowed {
            tracing::debug!("Ignoring message from non-allowed sender: {person_email}");
            return Ok(());
        }
    }

    if text.is_empty() {
        return Ok(());
    }

    // Build metadata for response routing
    let metadata = serde_json::json!({
        "roomId": room_id,
        "personEmail": person_email,
        "parentId": parent_id,
    });

    let incoming = IncomingMessage::new("webex", &person_email, &text)
        .with_metadata(metadata);

    let incoming = if let Some(pid) = parent_id {
        incoming.with_thread(pid)
    } else {
        incoming
    };

    let _ = tx.send(incoming).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webex_channel_name() {
        let config = WebexChannelConfig {
            token: "test".into(),
            webhook_port: 8080,
            allowed_sender: None,
        };
        let channel = WebexChannel::new(config);
        assert_eq!(channel.name(), "webex");
    }

    #[test]
    fn test_webex_config_from_env_missing_token() {
        // If WEBEX_TOKEN is not set in the environment, from_env should fail.
        // We cannot remove env vars (unsafe in edition 2024), so we only run
        // the assertion when the var is genuinely absent.
        if std::env::var("WEBEX_TOKEN").is_err() {
            let result = WebexChannelConfig::from_env();
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_webex_config_defaults() {
        // Test the config structure directly (no env mutation needed).
        let config = WebexChannelConfig {
            token: "test-token".into(),
            webhook_port: 8080,
            allowed_sender: None,
        };
        assert_eq!(config.token, "test-token");
        assert_eq!(config.webhook_port, 8080);
        assert!(config.allowed_sender.is_none());
    }

    #[test]
    fn test_webex_config_custom_port() {
        // Test the config structure with a custom port (no env mutation needed).
        let config = WebexChannelConfig {
            token: "t".into(),
            webhook_port: 9090,
            allowed_sender: None,
        };
        assert_eq!(config.webhook_port, 9090);
    }
}
