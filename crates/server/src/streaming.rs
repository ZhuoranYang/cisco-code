//! Server-Sent Events (SSE) streaming for job events.
//!
//! Clients connect to `/api/v1/jobs/{id}/stream` and receive real-time
//! StreamEvents as SSE messages.

use std::convert::Infallible;
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use futures::stream::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

use cisco_code_protocol::StreamEvent;

/// Convert a mpsc receiver of StreamEvents into an SSE stream.
pub fn stream_events(
    rx: mpsc::Receiver<StreamEvent>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let stream = ReceiverStream::new(rx).map(|event| {
        let event_type = match &event {
            StreamEvent::TurnStart { .. } => "turn_start",
            StreamEvent::TextDelta { .. } => "text_delta",
            StreamEvent::ToolUseStart { .. } => "tool_use_start",
            StreamEvent::ToolInputDelta { .. } => "tool_input_delta",
            StreamEvent::ToolExecutionStart { .. } => "tool_execution_start",
            StreamEvent::ToolProgress { .. } => "tool_progress",
            StreamEvent::ToolResult { .. } => "tool_result",
            StreamEvent::PermissionRequest { .. } => "permission_request",
            StreamEvent::TurnEnd { .. } => "turn_end",
            StreamEvent::Error { .. } => "error",
        };

        let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".into());
        Ok(Event::default().event(event_type).data(data))
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
}

/// Format a StreamEvent as a JSON string for WebSocket messages.
pub fn event_to_json(event: &StreamEvent) -> String {
    serde_json::to_string(event).unwrap_or_else(|_| r#"{"type":"error","message":"serialization failed"}"#.into())
}

/// Parse a StreamEvent from a JSON string.
pub fn json_to_event(json: &str) -> Result<StreamEvent, serde_json::Error> {
    serde_json::from_str(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cisco_code_protocol::{StopReason, TokenUsage};

    #[test]
    fn test_event_to_json_text_delta() {
        let event = StreamEvent::TextDelta {
            text: "hello world".into(),
        };
        let json = event_to_json(&event);
        assert!(json.contains("hello world"));
        assert!(json.contains("TextDelta") || json.contains("text"));
    }

    #[test]
    fn test_event_to_json_tool_result() {
        let event = StreamEvent::ToolResult {
            tool_use_id: "tu_1".into(),
            result: "file contents".into(),
            is_error: false,
        };
        let json = event_to_json(&event);
        assert!(json.contains("tu_1"));
        assert!(json.contains("file contents"));
    }

    #[test]
    fn test_json_roundtrip() {
        let event = StreamEvent::TurnEnd {
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
        };
        let json = event_to_json(&event);
        let parsed = json_to_event(&json).unwrap();
        match parsed {
            StreamEvent::TurnEnd { stop_reason, usage } => {
                assert_eq!(stop_reason, StopReason::EndTurn);
                assert_eq!(usage.input_tokens, 100);
            }
            _ => panic!("wrong event type"),
        }
    }

    #[test]
    fn test_json_to_event_error() {
        let result = json_to_event("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_event_to_json_error_event() {
        let event = StreamEvent::Error {
            message: "something broke".into(),
            recoverable: true,
        };
        let json = event_to_json(&event);
        assert!(json.contains("something broke"));
    }
}
