//! WebSocket handler for bidirectional session control.
//!
//! Provides a DirectConnect-style protocol where clients can:
//! - Send user messages
//! - Receive streaming events
//! - Cancel running jobs
//! - Query job status

use axum::extract::ws::{Message as WsMessage, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use futures::stream::StreamExt;
use futures::SinkExt;
use serde::{Deserialize, Serialize};

use cisco_code_protocol::StreamEvent;

use crate::state::AppState;

/// Client → Server messages over WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsClientMessage {
    /// Submit a new user message to the session.
    #[serde(rename = "user_message")]
    UserMessage {
        content: String,
        /// Optional attachments.
        #[serde(default)]
        attachments: Vec<WsAttachment>,
    },

    /// Cancel the current job.
    #[serde(rename = "cancel")]
    Cancel,

    /// Request job status.
    #[serde(rename = "status")]
    StatusRequest,

    /// Respond to a permission request.
    #[serde(rename = "permission_response")]
    PermissionResponse {
        tool_use_id: String,
        approved: bool,
    },

    /// Ping to keep connection alive.
    #[serde(rename = "ping")]
    Ping,
}

/// Server → Client messages over WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WsServerMessage {
    /// A stream event from the agent.
    #[serde(rename = "event")]
    Event { event: StreamEvent },

    /// Job status response.
    #[serde(rename = "status")]
    Status {
        job_id: Option<String>,
        status: String,
        turns: u32,
    },

    /// Session connected acknowledgement.
    #[serde(rename = "connected")]
    Connected {
        session_id: String,
        server_version: String,
    },

    /// Error message.
    #[serde(rename = "error")]
    Error { message: String, code: String },

    /// Pong response to ping.
    #[serde(rename = "pong")]
    Pong,
}

/// Attachment in a WebSocket message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsAttachment {
    pub filename: String,
    pub content: String,
}

/// Parse a client WebSocket message from JSON text.
pub fn parse_client_message(text: &str) -> Result<WsClientMessage, serde_json::Error> {
    serde_json::from_str(text)
}

/// Serialize a server message to JSON text.
pub fn serialize_server_message(msg: &WsServerMessage) -> String {
    serde_json::to_string(msg).unwrap_or_else(|_| {
        r#"{"type":"error","message":"serialization failed","code":"internal"}"#.into()
    })
}

/// Wrap a StreamEvent into a WsServerMessage.
pub fn wrap_event(event: StreamEvent) -> WsServerMessage {
    WsServerMessage::Event { event }
}

// ---------------------------------------------------------------------------
// Axum WebSocket handler
// ---------------------------------------------------------------------------

/// Axum handler: upgrade HTTP to WebSocket for a session.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws_session(socket, session_id, state))
}

/// Main WebSocket session loop.
///
/// Sends a `Connected` message, then processes client messages and forwards
/// agent events back to the client.
async fn handle_ws_session(socket: WebSocket, session_id: String, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Send connected acknowledgement
    let connected = WsServerMessage::Connected {
        session_id: session_id.clone(),
        server_version: state.version.clone(),
    };
    if sender
        .send(WsMessage::Text(serialize_server_message(&connected).into()))
        .await
        .is_err()
    {
        return;
    }

    // Track the current job for this WS session
    let mut current_job_id: Option<String> = None;

    while let Some(Ok(msg)) = receiver.next().await {
        let text = match msg {
            WsMessage::Text(t) => t.to_string(),
            WsMessage::Ping(_) => {
                let _ = sender.send(WsMessage::Pong(vec![].into())).await;
                continue;
            }
            WsMessage::Close(_) => break,
            _ => continue,
        };

        let client_msg = match parse_client_message(&text) {
            Ok(m) => m,
            Err(_) => {
                let err = WsServerMessage::Error {
                    message: "Invalid message format".into(),
                    code: "parse_error".into(),
                };
                let _ = sender
                    .send(WsMessage::Text(serialize_server_message(&err).into()))
                    .await;
                continue;
            }
        };

        match client_msg {
            WsClientMessage::UserMessage { content, .. } => {
                // Submit as a job and stream events back
                let request = crate::jobs::JobRequest {
                    prompt: content.clone(),
                    session_id: Some(session_id.clone()),
                    model: None,
                    max_turns: None,
                    cwd: None,
                };

                match state.jobs.submit(request).await {
                    Ok(job) => {
                        let job_id = job.id.clone();
                        current_job_id = Some(job_id.clone());

                        // Subscribe to events before spawning execution
                        let mut rx = match state.jobs.subscribe(&job_id, 256).await {
                            Ok(rx) => rx,
                            Err(_) => continue,
                        };

                        // Spawn execution
                        state.executor.spawn(
                            job_id.clone(),
                            content,
                            Some(session_id.clone()),
                            None,
                            None,
                        );

                        // Forward events to WebSocket
                        while let Some(event) = rx.recv().await {
                            let ws_msg = wrap_event(event);
                            let json = serialize_server_message(&ws_msg);
                            if sender.send(WsMessage::Text(json.into())).await.is_err() {
                                return; // Client disconnected
                            }
                        }
                    }
                    Err(e) => {
                        let err = WsServerMessage::Error {
                            message: e.to_string(),
                            code: "submit_failed".into(),
                        };
                        let _ = sender
                            .send(WsMessage::Text(serialize_server_message(&err).into()))
                            .await;
                    }
                }
            }

            WsClientMessage::Cancel => {
                if let Some(ref jid) = current_job_id {
                    let _ = state.jobs.cancel(jid).await;
                }
            }

            WsClientMessage::StatusRequest => {
                let status = if let Some(ref jid) = current_job_id {
                    if let Some(job) = state.jobs.get(jid).await {
                        WsServerMessage::Status {
                            job_id: Some(jid.clone()),
                            status: format!("{:?}", job.status),
                            turns: job.turns,
                        }
                    } else {
                        WsServerMessage::Status {
                            job_id: None,
                            status: "idle".into(),
                            turns: 0,
                        }
                    }
                } else {
                    WsServerMessage::Status {
                        job_id: None,
                        status: "idle".into(),
                        turns: 0,
                    }
                };
                let _ = sender
                    .send(WsMessage::Text(serialize_server_message(&status).into()))
                    .await;
            }

            WsClientMessage::Ping => {
                let pong = WsServerMessage::Pong;
                let _ = sender
                    .send(WsMessage::Text(serialize_server_message(&pong).into()))
                    .await;
            }

            WsClientMessage::PermissionResponse { .. } => {
                // TODO: Forward to runtime's permission callback (Phase 4)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_user_message() {
        let json = r#"{"type":"user_message","content":"hello","attachments":[]}"#;
        let msg = parse_client_message(json).unwrap();
        match msg {
            WsClientMessage::UserMessage { content, .. } => {
                assert_eq!(content, "hello");
            }
            _ => panic!("wrong message type"),
        }
    }

    #[test]
    fn test_parse_cancel() {
        let json = r#"{"type":"cancel"}"#;
        let msg = parse_client_message(json).unwrap();
        assert!(matches!(msg, WsClientMessage::Cancel));
    }

    #[test]
    fn test_parse_permission_response() {
        let json = r#"{"type":"permission_response","tool_use_id":"tu_1","approved":true}"#;
        let msg = parse_client_message(json).unwrap();
        match msg {
            WsClientMessage::PermissionResponse {
                tool_use_id,
                approved,
            } => {
                assert_eq!(tool_use_id, "tu_1");
                assert!(approved);
            }
            _ => panic!("wrong message type"),
        }
    }

    #[test]
    fn test_serialize_connected() {
        let msg = WsServerMessage::Connected {
            session_id: "sess-1".into(),
            server_version: "0.1.0".into(),
        };
        let json = serialize_server_message(&msg);
        assert!(json.contains("sess-1"));
        assert!(json.contains("connected"));
    }

    #[test]
    fn test_serialize_status() {
        let msg = WsServerMessage::Status {
            job_id: Some("job-1".into()),
            status: "running".into(),
            turns: 3,
        };
        let json = serialize_server_message(&msg);
        assert!(json.contains("running"));
        assert!(json.contains("job-1"));
    }

    #[test]
    fn test_serialize_error() {
        let msg = WsServerMessage::Error {
            message: "not found".into(),
            code: "not_found".into(),
        };
        let json = serialize_server_message(&msg);
        assert!(json.contains("not found"));
        assert!(json.contains("not_found"));
    }

    #[test]
    fn test_wrap_event() {
        let event = StreamEvent::TextDelta {
            text: "hello".into(),
        };
        let wrapped = wrap_event(event);
        match wrapped {
            WsServerMessage::Event { event } => {
                assert!(matches!(event, StreamEvent::TextDelta { .. }));
            }
            _ => panic!("wrong message type"),
        }
    }

    #[test]
    fn test_parse_ping() {
        let json = r#"{"type":"ping"}"#;
        let msg = parse_client_message(json).unwrap();
        assert!(matches!(msg, WsClientMessage::Ping));
    }

    #[test]
    fn test_pong_serialization() {
        let msg = WsServerMessage::Pong;
        let json = serialize_server_message(&msg);
        assert!(json.contains("pong"));
    }

    #[test]
    fn test_parse_invalid_json() {
        let result = parse_client_message("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_user_message_with_attachments() {
        let json = r#"{"type":"user_message","content":"check this","attachments":[{"filename":"test.py","content":"print('hi')"}]}"#;
        let msg = parse_client_message(json).unwrap();
        match msg {
            WsClientMessage::UserMessage {
                content,
                attachments,
            } => {
                assert_eq!(content, "check this");
                assert_eq!(attachments.len(), 1);
                assert_eq!(attachments[0].filename, "test.py");
            }
            _ => panic!("wrong type"),
        }
    }
}
