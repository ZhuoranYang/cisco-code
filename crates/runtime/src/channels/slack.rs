//! Slack channel — receive messages from Slack, respond back.
//!
//! Uses Slack Events API: Slack sends HTTP POST to our webhook URL when
//! someone messages or @mentions the bot. We parse the event, feed the
//! message to the agent, and post the response back via chat.postMessage.
//!
//! Auth: SLACK_TOKEN (Bot OAuth Token xoxb-...), SLACK_SIGNING_SECRET.
//! Setup: Create a Slack app, enable Events API, subscribe to
//! message.im and app_mention events.

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use super::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};

const SLACK_API_BASE: &str = "https://slack.com/api";

/// Slack channel configuration.
#[derive(Debug, Clone)]
pub struct SlackChannelConfig {
    /// Slack Bot OAuth Token (xoxb-...).
    pub token: String,
    /// Slack signing secret for request verification.
    pub signing_secret: Option<String>,
    /// Port to listen for events on.
    pub webhook_port: u16,
}

impl SlackChannelConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        let token = std::env::var("SLACK_TOKEN")
            .map_err(|_| anyhow::anyhow!("SLACK_TOKEN not set"))?;
        let signing_secret = std::env::var("SLACK_SIGNING_SECRET").ok();
        let port = std::env::var("SLACK_WEBHOOK_PORT")
            .unwrap_or_else(|_| "8081".into())
            .parse()
            .unwrap_or(8081);

        Ok(Self {
            token,
            signing_secret,
            webhook_port: port,
        })
    }
}

/// Slack channel that receives Events API webhooks.
pub struct SlackChannel {
    config: SlackChannelConfig,
    http: reqwest::Client,
}

impl SlackChannel {
    pub fn new(config: SlackChannelConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    /// Post a message to a Slack channel/DM.
    async fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut body = serde_json::json!({
            "channel": channel,
            "text": text,
        });

        if let Some(ts) = thread_ts {
            body["thread_ts"] = serde_json::json!(ts);
        }

        let resp = self
            .http
            .post(format!("{SLACK_API_BASE}/chat.postMessage"))
            .bearer_auth(&self.config.token)
            .json(&body)
            .send()
            .await?;

        let result: serde_json::Value = resp.json().await?;
        if !result["ok"].as_bool().unwrap_or(false) {
            let err = result["error"].as_str().unwrap_or("unknown");
            anyhow::bail!("Slack API error: {err}");
        }

        Ok(())
    }

    /// Get the bot's own user ID to filter self-messages.
    async fn get_bot_user_id(&self) -> anyhow::Result<String> {
        let resp = self
            .http
            .post(format!("{SLACK_API_BASE}/auth.test"))
            .bearer_auth(&self.config.token)
            .send()
            .await?;

        let body: serde_json::Value = resp.json().await?;
        Ok(body["user_id"].as_str().unwrap_or("").to_string())
    }
}

impl Channel for SlackChannel {
    fn name(&self) -> &str {
        "slack"
    }

    async fn start(&self) -> anyhow::Result<MessageStream> {
        let (tx, rx) = mpsc::channel(32);
        let port = self.config.webhook_port;
        let bot_user_id = self.get_bot_user_id().await.unwrap_or_default();

        tokio::spawn(async move {
            let listener = match tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("Failed to bind Slack webhook on port {port}: {e}");
                    return;
                }
            };

            tracing::info!("Slack webhook listening on port {port}");

            loop {
                let (stream, _addr) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(e) => {
                        tracing::error!("Slack webhook accept error: {e}");
                        continue;
                    }
                };

                let tx = tx.clone();
                let bot_user_id = bot_user_id.clone();

                tokio::spawn(async move {
                    if let Err(e) =
                        handle_slack_webhook(stream, tx, &bot_user_id).await
                    {
                        tracing::debug!("Slack webhook error: {e}");
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
        let channel_id = msg.metadata["channel"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing channel in message metadata"))?;

        let thread_ts = response
            .thread_id
            .as_deref()
            .or_else(|| msg.metadata["thread_ts"].as_str());

        self.post_message(channel_id, &response.content, thread_ts)
            .await
    }

    async fn health_check(&self) -> anyhow::Result<()> {
        let resp = self
            .http
            .post(format!("{SLACK_API_BASE}/auth.test"))
            .bearer_auth(&self.config.token)
            .send()
            .await?;

        let body: serde_json::Value = resp.json().await?;
        if body["ok"].as_bool().unwrap_or(false) {
            Ok(())
        } else {
            let err = body["error"].as_str().unwrap_or("unknown");
            anyhow::bail!("Slack health check failed: {err}")
        }
    }
}

/// Handle a single Slack webhook HTTP connection.
async fn handle_slack_webhook(
    mut stream: tokio::net::TcpStream,
    tx: mpsc::Sender<IncomingMessage>,
    bot_user_id: &str,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut buf = vec![0u8; 16384];
    let n = stream.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let body = request.split("\r\n\r\n").nth(1).unwrap_or("");

    let payload: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => {
            let response = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";
            stream.write_all(response.as_bytes()).await?;
            return Ok(());
        }
    };

    let event_type = payload["type"].as_str().unwrap_or("");

    match event_type {
        // URL verification challenge (Slack setup handshake)
        "url_verification" => {
            let challenge = payload["challenge"].as_str().unwrap_or("");
            let resp_body = serde_json::json!({"challenge": challenge}).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{resp_body}",
                resp_body.len()
            );
            stream.write_all(response.as_bytes()).await?;
        }

        // Event callback (actual messages)
        "event_callback" => {
            // Respond 200 immediately
            let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
            stream.write_all(response.as_bytes()).await?;

            if let Some(event) = payload.get("event") {
                let event_type = event["type"].as_str().unwrap_or("");
                let user = event["user"].as_str().unwrap_or("");
                let text = event["text"].as_str().unwrap_or("").to_string();
                let channel = event["channel"].as_str().unwrap_or("").to_string();
                let ts = event["ts"].as_str().unwrap_or("").to_string();
                let thread_ts = event["thread_ts"].as_str().map(|s| s.to_string());

                // Skip bot's own messages
                if user == bot_user_id {
                    return Ok(());
                }

                // Skip bot messages and subtypes
                if event["bot_id"].is_string() || event["subtype"].is_string() {
                    return Ok(());
                }

                // Handle app_mention and DMs
                let should_process = match event_type {
                    "app_mention" => true,
                    "message" => channel.starts_with('D'), // DMs only
                    _ => false,
                };

                if should_process && !text.is_empty() {
                    let cleaned = strip_slack_mention(&text);

                    let metadata = serde_json::json!({
                        "channel": channel,
                        "ts": ts,
                        "thread_ts": thread_ts,
                    });

                    let mut incoming =
                        IncomingMessage::new("slack", user, &cleaned).with_metadata(metadata);

                    if let Some(tts) = thread_ts.or(Some(ts)) {
                        incoming = incoming.with_thread(tts);
                    }

                    let _ = tx.send(incoming).await;
                }
            }
        }

        _ => {
            let response = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
            stream.write_all(response.as_bytes()).await?;
        }
    }

    Ok(())
}

/// Strip leading @mention from Slack message text.
/// Slack mentions look like `<@U12345678> the message`.
fn strip_slack_mention(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.starts_with("<@") {
        if let Some(end) = trimmed.find('>') {
            return trimmed[end + 1..].trim_start().to_string();
        }
    }
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slack_channel_name() {
        let config = SlackChannelConfig {
            token: "xoxb-test".into(),
            signing_secret: None,
            webhook_port: 8081,
        };
        let channel = SlackChannel::new(config);
        assert_eq!(channel.name(), "slack");
    }

    #[test]
    fn test_slack_config_from_env() {
        std::env::remove_var("SLACK_TOKEN");
        assert!(SlackChannelConfig::from_env().is_err());

        std::env::set_var("SLACK_TOKEN", "xoxb-test");
        let config = SlackChannelConfig::from_env().unwrap();
        assert_eq!(config.token, "xoxb-test");
        assert_eq!(config.webhook_port, 8081);
        std::env::remove_var("SLACK_TOKEN");
    }

    #[test]
    fn test_strip_slack_mention() {
        assert_eq!(strip_slack_mention("<@U123> hello"), "hello");
        assert_eq!(strip_slack_mention("<@U123>hello"), "hello");
        assert_eq!(strip_slack_mention("no mention"), "no mention");
        assert_eq!(strip_slack_mention("  <@U123>  hi  "), "hi");
        assert_eq!(strip_slack_mention("<@UBOT123> what's up?"), "what's up?");
    }

    #[test]
    fn test_strip_slack_mention_empty() {
        assert_eq!(strip_slack_mention(""), "");
        assert_eq!(strip_slack_mention("<@U123>"), "");
    }
}
