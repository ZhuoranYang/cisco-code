//! SendMessage tool — send messages to agents or teammates.
//!
//! Matches Claude Code's SendMessageTool: enables inter-agent
//! communication in swarm/team mode. Supports plain text,
//! broadcast, and structured protocol messages.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

pub struct SendMessageTool;

const VALID_STRUCTURED_TYPES: &[&str] = &[
    "shutdown_request",
    "shutdown_response",
    "plan_approval_response",
];

impl Tool for SendMessageTool {
    fn name(&self) -> &str {
        "SendMessage"
    }

    fn description(&self) -> &str {
        "Send a message to another agent or teammate. Use '*' as recipient to broadcast. Supports plain text and structured protocol messages."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "to": {
                    "type": "string",
                    "description": "Recipient: agent name, '*' for broadcast, or 'bridge:<id>' for remote"
                },
                "message": {
                    "type": "string",
                    "description": "Message content (plain text or structured JSON)"
                },
                "summary": {
                    "type": "string",
                    "description": "5-10 word preview of the message (required for plain text)"
                }
            },
            "required": ["to", "message"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let to = match input["to"].as_str() {
            Some(t) if !t.trim().is_empty() => t.trim(),
            Some(_) => return Ok(ToolResult::error("'to' must not be empty")),
            None => return Ok(ToolResult::error("missing required parameter 'to'")),
        };

        let message = match input["message"].as_str() {
            Some(m) if !m.trim().is_empty() => m,
            Some(_) => return Ok(ToolResult::error("'message' must not be empty")),
            None => return Ok(ToolResult::error("missing required parameter 'message'")),
        };

        let summary = input.get("summary").and_then(|v| v.as_str());

        // Determine if this is a broadcast
        let is_broadcast = to == "*";

        // Determine if this is a structured message
        let is_structured = serde_json::from_str::<serde_json::Value>(message)
            .ok()
            .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(String::from))
            .is_some_and(|t| VALID_STRUCTURED_TYPES.contains(&t.as_str()));

        // Plain text messages require a summary
        if !is_structured && summary.is_none() {
            return Ok(ToolResult::error(
                "plain text messages require a 'summary' parameter (5-10 word preview)",
            ));
        }

        let request = json!({
            "type": "send_message",
            "to": to,
            "message": message,
            "summary": summary,
            "is_broadcast": is_broadcast,
            "is_structured": is_structured,
        });

        Ok(ToolResult::success(
            serde_json::to_string_pretty(&request)
                .unwrap_or_else(|_| request.to_string()),
        ))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolContext;

    fn ctx() -> ToolContext {
        ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
            progress_tx: None,
        }
    }

    #[tokio::test]
    async fn test_send_message_plain_text() {
        let tool = SendMessageTool;
        let result = tool
            .call(
                json!({
                    "to": "agent-1",
                    "message": "Please check the test results",
                    "summary": "Check test results"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["type"], "send_message");
        assert_eq!(parsed["to"], "agent-1");
        assert_eq!(parsed["is_broadcast"], false);
        assert_eq!(parsed["is_structured"], false);
    }

    #[tokio::test]
    async fn test_send_message_broadcast() {
        let tool = SendMessageTool;
        let result = tool
            .call(
                json!({
                    "to": "*",
                    "message": "All agents stop work",
                    "summary": "Stop all work"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["is_broadcast"], true);
    }

    #[tokio::test]
    async fn test_send_message_structured() {
        let tool = SendMessageTool;
        let structured = json!({"type": "shutdown_request", "reason": "done"}).to_string();
        let result = tool
            .call(
                json!({
                    "to": "agent-2",
                    "message": structured,
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["is_structured"], true);
    }

    #[tokio::test]
    async fn test_send_message_plain_requires_summary() {
        let tool = SendMessageTool;
        let result = tool
            .call(
                json!({
                    "to": "agent-1",
                    "message": "Hello there"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("summary"));
    }

    #[tokio::test]
    async fn test_send_message_missing_to() {
        let tool = SendMessageTool;
        let result = tool
            .call(json!({"message": "hello", "summary": "hi"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_send_message_missing_message() {
        let tool = SendMessageTool;
        let result = tool
            .call(json!({"to": "agent-1"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_send_message_empty_to() {
        let tool = SendMessageTool;
        let result = tool
            .call(json!({"to": "", "message": "x", "summary": "x"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[test]
    fn test_send_message_schema() {
        let tool = SendMessageTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("to")));
        assert!(required.contains(&json!("message")));
    }

    #[test]
    fn test_send_message_permission() {
        assert_eq!(SendMessageTool.permission_level(), PermissionLevel::Execute);
    }
}
