//! Async channel utilities for event streaming.
//!
//! Provides typed channels for streaming agent events between the
//! runtime and the UI/API layer.

use cisco_code_protocol::StreamEvent;
use tokio::sync::mpsc;

/// Create a new event channel pair.
///
/// Returns (sender, receiver) for streaming `StreamEvent`s from the
/// runtime to the rendering layer.
pub fn event_channel(buffer: usize) -> (EventSender, EventReceiver) {
    let (tx, rx) = mpsc::channel(buffer);
    (EventSender(tx), EventReceiver(rx))
}

/// Sender side of the event channel.
#[derive(Clone)]
pub struct EventSender(mpsc::Sender<StreamEvent>);

impl EventSender {
    pub async fn send(&self, event: StreamEvent) -> Result<(), StreamEvent> {
        self.0.send(event).await.map_err(|e| e.0)
    }

    pub fn try_send(&self, event: StreamEvent) -> Result<(), StreamEvent> {
        self.0.try_send(event).map_err(|e| match e {
            mpsc::error::TrySendError::Full(e) | mpsc::error::TrySendError::Closed(e) => e,
        })
    }
}

/// Receiver side of the event channel.
pub struct EventReceiver(mpsc::Receiver<StreamEvent>);

impl EventReceiver {
    pub async fn recv(&mut self) -> Option<StreamEvent> {
        self.0.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cisco_code_protocol::{StopReason, TokenUsage};

    #[tokio::test]
    async fn test_event_channel() {
        let (tx, mut rx) = event_channel(16);

        tx.send(StreamEvent::TextDelta {
            text: "hello".into(),
        })
        .await
        .unwrap();

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, StreamEvent::TextDelta { ref text } if text == "hello"));
    }

    #[tokio::test]
    async fn test_event_channel_multiple() {
        let (tx, mut rx) = event_channel(16);

        tx.send(StreamEvent::TurnStart {
            model: "test".into(),
            turn_number: 1,
        })
        .await
        .unwrap();

        tx.send(StreamEvent::TextDelta {
            text: "hi".into(),
        })
        .await
        .unwrap();

        tx.send(StreamEvent::TurnEnd {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage::default(),
        })
        .await
        .unwrap();

        let mut events = Vec::new();
        while let Ok(Some(e)) = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            rx.recv(),
        )
        .await
        {
            events.push(e);
        }

        assert_eq!(events.len(), 3);
    }
}
