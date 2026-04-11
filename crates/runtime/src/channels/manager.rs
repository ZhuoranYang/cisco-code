//! Channel manager for coordinating multiple input channels.

use std::collections::HashMap;
use std::sync::Arc;

use futures::stream;
use tokio::sync::RwLock;

use super::{Channel, IncomingMessage, MessageStream, OutgoingResponse, StatusUpdate};

/// Manages multiple input channels and merges their message streams.
pub struct ChannelManager {
    channels: Arc<RwLock<HashMap<String, Arc<dyn Channel>>>>,
}

impl ChannelManager {
    pub fn new() -> Self {
        Self {
            channels: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a channel to the manager.
    pub async fn add(&self, channel: Box<dyn Channel>) {
        let name = channel.name().to_string();
        self.channels
            .write()
            .await
            .insert(name.clone(), Arc::from(channel));
        tracing::debug!("Added channel: {}", name);
    }

    /// Start all channels and return a merged stream of messages.
    pub async fn start_all(&self) -> anyhow::Result<MessageStream> {
        let channels = self.channels.read().await;
        let mut streams: Vec<MessageStream> = Vec::new();

        for (name, channel) in channels.iter() {
            match channel.start().await {
                Ok(stream) => {
                    tracing::debug!("Started channel: {}", name);
                    streams.push(stream);
                }
                Err(e) => {
                    tracing::error!("Failed to start channel {}: {}", name, e);
                }
            }
        }

        if streams.is_empty() {
            anyhow::bail!("No channels started successfully");
        }

        let merged = stream::select_all(streams);
        Ok(Box::pin(merged))
    }

    /// Send a response to the channel that sent the original message.
    pub async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> anyhow::Result<()> {
        let channels = self.channels.read().await;
        if let Some(channel) = channels.get(&msg.channel) {
            channel.respond(msg, response).await
        } else {
            anyhow::bail!("Channel not found: {}", msg.channel)
        }
    }

    /// Send a status update to a specific channel.
    pub async fn send_status(
        &self,
        channel_name: &str,
        status: StatusUpdate,
        metadata: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let channels = self.channels.read().await;
        if let Some(channel) = channels.get(channel_name) {
            channel.send_status(status, metadata).await
        } else {
            Ok(()) // status is best-effort
        }
    }

    /// Check health of all channels.
    pub async fn health_check_all(&self) -> HashMap<String, anyhow::Result<()>> {
        let channels = self.channels.read().await;
        let mut results = HashMap::new();
        for (name, channel) in channels.iter() {
            results.insert(name.clone(), channel.health_check().await);
        }
        results
    }

    /// Shutdown all channels gracefully.
    pub async fn shutdown_all(&self) -> anyhow::Result<()> {
        let channels = self.channels.read().await;
        for (name, channel) in channels.iter() {
            if let Err(e) = channel.shutdown().await {
                tracing::error!("Error shutting down channel {}: {}", name, e);
            }
        }
        Ok(())
    }

    /// List registered channel names.
    pub async fn channel_names(&self) -> Vec<String> {
        self.channels.read().await.keys().cloned().collect()
    }
}

impl Default for ChannelManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::IncomingMessage;
    use futures::StreamExt;
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    /// A test channel that sends messages via an mpsc sender.
    struct StubChannel {
        name: String,
        rx: tokio::sync::Mutex<Option<mpsc::Receiver<IncomingMessage>>>,
    }

    impl StubChannel {
        fn new(name: &str) -> (Self, mpsc::Sender<IncomingMessage>) {
            let (tx, rx) = mpsc::channel(32);
            (
                Self {
                    name: name.to_string(),
                    rx: tokio::sync::Mutex::new(Some(rx)),
                },
                tx,
            )
        }
    }

    #[async_trait::async_trait]
    impl Channel for StubChannel {
        fn name(&self) -> &str {
            &self.name
        }

        async fn start(&self) -> anyhow::Result<MessageStream> {
            let rx = self
                .rx
                .lock()
                .await
                .take()
                .ok_or_else(|| anyhow::anyhow!("already started"))?;
            Ok(Box::pin(ReceiverStream::new(rx)))
        }

        async fn respond(
            &self,
            _msg: &IncomingMessage,
            _response: OutgoingResponse,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn health_check(&self) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_add_and_start_all() {
        let manager = ChannelManager::new();
        let (stub, sender) = StubChannel::new("test");
        manager.add(Box::new(stub)).await;

        let mut stream = manager.start_all().await.unwrap();

        sender
            .send(IncomingMessage::new("test", "user1", "hello"))
            .await
            .unwrap();

        let msg = stream.next().await.unwrap();
        assert_eq!(msg.content, "hello");
        assert_eq!(msg.channel, "test");
    }

    #[tokio::test]
    async fn test_start_all_no_channels_errors() {
        let manager = ChannelManager::new();
        let result = manager.start_all().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_respond_unknown_channel_errors() {
        let manager = ChannelManager::new();
        let msg = IncomingMessage::new("nonexistent", "user1", "test");
        let result = manager.respond(&msg, OutgoingResponse::text("hi")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_multiple_channels_merged() {
        let manager = ChannelManager::new();
        let (stub1, tx1) = StubChannel::new("alpha");
        let (stub2, tx2) = StubChannel::new("beta");
        manager.add(Box::new(stub1)).await;
        manager.add(Box::new(stub2)).await;

        let mut stream = manager.start_all().await.unwrap();

        tx1.send(IncomingMessage::new("alpha", "u1", "from alpha"))
            .await
            .unwrap();
        tx2.send(IncomingMessage::new("beta", "u2", "from beta"))
            .await
            .unwrap();

        let mut messages = Vec::new();
        messages.push(stream.next().await.unwrap());
        messages.push(stream.next().await.unwrap());

        let contents: Vec<&str> = messages.iter().map(|m| m.content.as_str()).collect();
        assert!(contents.contains(&"from alpha"));
        assert!(contents.contains(&"from beta"));
    }
}
