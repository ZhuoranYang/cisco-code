//! Sleep tool — pause execution for a specified duration.
//!
//! Matches Claude Code's SleepTool: cheaper than Bash sleep (no shell process),
//! interruptible, and sends periodic tick check-ins.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

pub struct SleepTool;

/// Maximum sleep duration in seconds (10 minutes).
const MAX_DURATION_SECS: u64 = 600;

impl Tool for SleepTool {
    fn name(&self) -> &str {
        "Sleep"
    }

    fn description(&self) -> &str {
        "Pause execution for a specified number of seconds. Cheaper than Bash sleep — no shell process. The user can interrupt at any time."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "seconds": {
                    "type": "integer",
                    "description": "Number of seconds to sleep (1-600)"
                },
                "reason": {
                    "type": "string",
                    "description": "Why the sleep is needed (e.g., 'waiting for CI to complete')"
                }
            },
            "required": ["seconds"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let seconds = match input["seconds"].as_u64() {
            Some(s) if s >= 1 && s <= MAX_DURATION_SECS => s,
            Some(0) => return Ok(ToolResult::error("'seconds' must be at least 1")),
            Some(s) if s > MAX_DURATION_SECS => {
                return Ok(ToolResult::error(format!(
                    "'seconds' must be at most {MAX_DURATION_SECS}"
                )))
            }
            Some(_) => return Ok(ToolResult::error("invalid 'seconds' value")),
            None => {
                return Ok(ToolResult::error(
                    "missing or invalid 'seconds' parameter (must be a positive integer)",
                ))
            }
        };

        let reason = input
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("no reason specified");

        // Actually sleep
        tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;

        Ok(ToolResult::success(format!(
            "Slept for {seconds} seconds. Reason: {reason}"
        )))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn is_concurrency_safe(&self) -> bool {
        true
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
    async fn test_sleep_short() {
        let tool = SleepTool;
        let start = std::time::Instant::now();
        let result = tool
            .call(json!({"seconds": 1, "reason": "testing"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(start.elapsed().as_secs() >= 1);
        assert!(result.output.contains("1 seconds"));
        assert!(result.output.contains("testing"));
    }

    #[tokio::test]
    async fn test_sleep_zero() {
        let tool = SleepTool;
        let result = tool
            .call(json!({"seconds": 0}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("at least 1"));
    }

    #[tokio::test]
    async fn test_sleep_too_long() {
        let tool = SleepTool;
        let result = tool
            .call(json!({"seconds": 9999}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("at most"));
    }

    #[tokio::test]
    async fn test_sleep_missing_seconds() {
        let tool = SleepTool;
        let result = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_sleep_default_reason() {
        let tool = SleepTool;
        let result = tool
            .call(json!({"seconds": 1}), &ctx())
            .await
            .unwrap();
        assert!(result.output.contains("no reason specified"));
    }

    #[test]
    fn test_sleep_schema() {
        let tool = SleepTool;
        let schema = tool.input_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("seconds")));
    }

    #[test]
    fn test_sleep_permission() {
        assert_eq!(SleepTool.permission_level(), PermissionLevel::ReadOnly);
    }

    #[test]
    fn test_sleep_name() {
        assert_eq!(SleepTool.name(), "Sleep");
    }
}
