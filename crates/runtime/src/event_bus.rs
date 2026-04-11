//! Internal event bus for daemon mode.
//!
//! The event bus is the spine of the daemon: channels, cron, and webhooks
//! push `TriggerEvent`s into it, and the daemon main loop consumes them to
//! route work into the right session/runtime.
//!
//! ```text
//! ┌──────────┐   ┌──────────┐   ┌──────────┐
//! │  Slack   │   │  Webex   │   │  CronMgr │  ...
//! └────┬─────┘   └────┬─────┘   └────┬─────┘
//!      │              │              │
//!      ▼              ▼              ▼
//!    ┌──────────────────────────────────┐
//!    │           EventBus (mpsc)        │
//!    └───────────────┬──────────────────┘
//!                    │
//!                    ▼
//!             Daemon main loop
//!                    │
//!       ┌────────────┼────────────┐
//!       ▼            ▼            ▼
//!   SessionRouter  Runtime    channel.respond()
//! ```

use tokio::sync::mpsc;

use crate::channels::IncomingMessage;
use crate::cron::CronJob;

/// Events that trigger agent work in daemon mode.
#[derive(Debug, Clone)]
pub enum TriggerEvent {
    /// A message arrived from an external channel (Slack, Webex, REPL, etc.).
    ChannelMessage(IncomingMessage),

    /// A cron job fired and needs execution.
    CronFired {
        /// The cron job definition (includes prompt, model, cwd).
        job: CronJob,
    },

    /// An HTTP webhook was received (future — Notion, GitHub, etc.).
    WebhookReceived {
        /// Source identifier (e.g., "github", "notion").
        source: String,
        /// Raw JSON payload.
        payload: serde_json::Value,
    },

    /// Graceful shutdown requested.
    Shutdown,
}

/// Sending half of the event bus — cloned into each producer (channel, cron, webhook).
pub type EventSender = mpsc::Sender<TriggerEvent>;

/// Receiving half — consumed by the daemon main loop.
pub type EventReceiver = mpsc::Receiver<TriggerEvent>;

/// Create an event bus with the given buffer capacity.
///
/// Returns `(sender, receiver)`. Clone `sender` for each producer.
/// The daemon loop owns the single `receiver`.
pub fn event_bus(capacity: usize) -> (EventSender, EventReceiver) {
    mpsc::channel(capacity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_channel_message_roundtrip() {
        let (tx, mut rx) = event_bus(16);

        let msg = IncomingMessage::new("slack", "U123", "hello daemon");
        tx.send(TriggerEvent::ChannelMessage(msg)).await.unwrap();

        match rx.recv().await.unwrap() {
            TriggerEvent::ChannelMessage(m) => {
                assert_eq!(m.content, "hello daemon");
                assert_eq!(m.channel, "slack");
            }
            _ => panic!("wrong event type"),
        }
    }

    #[tokio::test]
    async fn test_shutdown_event() {
        let (tx, mut rx) = event_bus(16);
        tx.send(TriggerEvent::Shutdown).await.unwrap();

        match rx.recv().await.unwrap() {
            TriggerEvent::Shutdown => {}
            _ => panic!("expected Shutdown"),
        }
    }

    #[tokio::test]
    async fn test_multiple_producers() {
        let (tx, mut rx) = event_bus(16);
        let tx2 = tx.clone();

        tx.send(TriggerEvent::ChannelMessage(
            IncomingMessage::new("slack", "u1", "from slack"),
        ))
        .await
        .unwrap();

        tx2.send(TriggerEvent::ChannelMessage(
            IncomingMessage::new("webex", "u2", "from webex"),
        ))
        .await
        .unwrap();

        let e1 = rx.recv().await.unwrap();
        let e2 = rx.recv().await.unwrap();

        let mut sources = Vec::new();
        if let TriggerEvent::ChannelMessage(m) = e1 {
            sources.push(m.channel);
        }
        if let TriggerEvent::ChannelMessage(m) = e2 {
            sources.push(m.channel);
        }
        assert!(sources.contains(&"slack".to_string()));
        assert!(sources.contains(&"webex".to_string()));
    }

    #[tokio::test]
    async fn test_webhook_event() {
        let (tx, mut rx) = event_bus(16);
        tx.send(TriggerEvent::WebhookReceived {
            source: "github".into(),
            payload: serde_json::json!({"action": "push"}),
        })
        .await
        .unwrap();

        match rx.recv().await.unwrap() {
            TriggerEvent::WebhookReceived { source, payload } => {
                assert_eq!(source, "github");
                assert_eq!(payload["action"], "push");
            }
            _ => panic!("wrong event type"),
        }
    }
}
