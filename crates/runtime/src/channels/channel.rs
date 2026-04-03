//! Channel trait and message types.
//!
//! Inspired by IronClaw's channel system: every input source (CLI, Webex, Slack)
//! implements the same trait. The agent loop is channel-agnostic.

use std::pin::Pin;

use futures::Stream;
use uuid::Uuid;

/// A message received from an external channel.
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    /// Unique message ID.
    pub id: Uuid,
    /// Channel this message came from (e.g., "repl", "webex", "slack").
    pub channel: String,
    /// User identifier within the channel.
    pub user_id: String,
    /// Optional display name.
    pub user_name: Option<String>,
    /// Message content.
    pub content: String,
    /// Thread/conversation ID for threaded conversations.
    pub thread_id: Option<String>,
    /// Channel-specific metadata (e.g., Slack channel ID, Webex room ID).
    pub metadata: serde_json::Value,
}

impl IncomingMessage {
    /// Create a new incoming message.
    pub fn new(
        channel: impl Into<String>,
        user_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            channel: channel.into(),
            user_id: user_id.into(),
            user_name: None,
            content: content.into(),
            thread_id: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_thread(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn with_user_name(mut self, name: impl Into<String>) -> Self {
        self.user_name = Some(name.into());
        self
    }
}

/// Stream of incoming messages from a channel.
pub type MessageStream = Pin<Box<dyn Stream<Item = IncomingMessage> + Send>>;

/// Response to send back to a channel.
#[derive(Debug, Clone)]
pub struct OutgoingResponse {
    /// The content to send.
    pub content: String,
    /// Optional thread ID to reply in.
    pub thread_id: Option<String>,
    /// Channel-specific metadata for routing.
    pub metadata: serde_json::Value,
}

impl OutgoingResponse {
    /// Create a simple text response.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            thread_id: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn in_thread(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }
}

/// Status updates for showing agent activity.
#[derive(Debug, Clone)]
pub enum StatusUpdate {
    /// Agent is thinking/processing.
    Thinking(String),
    /// Tool execution started.
    ToolStarted { name: String },
    /// Tool execution completed.
    ToolCompleted { name: String, success: bool },
    /// Streaming text chunk.
    StreamChunk(String),
    /// General status message.
    Status(String),
}

/// Trait for message channels.
///
/// Every input source implements this trait. The agent loop consumes
/// messages from all channels uniformly.
#[allow(async_fn_in_trait)]
pub trait Channel: Send + Sync {
    /// Get the channel name (e.g., "repl", "webex", "slack").
    fn name(&self) -> &str;

    /// Start listening for messages. Returns a stream of incoming messages.
    async fn start(&self) -> anyhow::Result<MessageStream>;

    /// Send a response back to the user in the context of the original message.
    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> anyhow::Result<()>;

    /// Send a status update (thinking, tool execution, etc.).
    /// Default: no-op for channels that don't support status.
    async fn send_status(
        &self,
        _status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// Check if the channel is healthy/connected.
    async fn health_check(&self) -> anyhow::Result<()>;

    /// Gracefully shut down the channel.
    async fn shutdown(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incoming_message_new() {
        let msg = IncomingMessage::new("repl", "user1", "hello");
        assert_eq!(msg.channel, "repl");
        assert_eq!(msg.user_id, "user1");
        assert_eq!(msg.content, "hello");
        assert!(msg.thread_id.is_none());
        assert!(msg.user_name.is_none());
    }

    #[test]
    fn test_incoming_message_builder() {
        let msg = IncomingMessage::new("slack", "U123", "hi")
            .with_thread("thread_1")
            .with_user_name("alice")
            .with_metadata(serde_json::json!({"channel_id": "C456"}));

        assert_eq!(msg.thread_id.as_deref(), Some("thread_1"));
        assert_eq!(msg.user_name.as_deref(), Some("alice"));
        assert_eq!(msg.metadata["channel_id"], "C456");
    }

    #[test]
    fn test_outgoing_response_text() {
        let resp = OutgoingResponse::text("hello back");
        assert_eq!(resp.content, "hello back");
        assert!(resp.thread_id.is_none());
    }

    #[test]
    fn test_outgoing_response_in_thread() {
        let resp = OutgoingResponse::text("reply").in_thread("t_123");
        assert_eq!(resp.thread_id.as_deref(), Some("t_123"));
    }
}
