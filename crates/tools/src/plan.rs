//! Plan mode tools — EnterPlanMode and ExitPlanMode.
//!
//! Matches Claude Code's plan mode: a mode where the agent focuses on
//! planning and designing before implementation. No code changes
//! are made in plan mode.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

/// Enter plan mode — switch to planning/design focus.
pub struct EnterPlanModeTool;

impl Tool for EnterPlanModeTool {
    fn name(&self) -> &str {
        "EnterPlanMode"
    }

    fn description(&self) -> &str {
        "Enter plan mode to focus on designing an implementation plan. In plan mode, no code changes are made — only research, analysis, and planning."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "Optional description of what to plan"
                }
            },
            "required": []
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let description = input["description"].as_str();

        let mut request = json!({
            "action": "enter_plan_mode",
        });

        if let Some(desc) = description {
            request["description"] = json!(desc);
        }

        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
}

/// Exit plan mode — return to normal execution.
pub struct ExitPlanModeTool;

impl Tool for ExitPlanModeTool {
    fn name(&self) -> &str {
        "ExitPlanMode"
    }

    fn description(&self) -> &str {
        "Exit plan mode and return to normal execution mode where code changes can be made."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn call(&self, _input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let request = json!({
            "action": "exit_plan_mode",
        });

        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
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
    async fn test_enter_plan_mode() {
        let tool = EnterPlanModeTool;
        let result = tool
            .call(json!({"description": "Complex refactoring"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("enter_plan_mode"));
        assert!(result.output.contains("Complex refactoring"));
    }

    #[tokio::test]
    async fn test_enter_plan_mode_no_reason() {
        let tool = EnterPlanModeTool;
        let result = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("enter_plan_mode"));
    }

    #[tokio::test]
    async fn test_exit_plan_mode() {
        let tool = ExitPlanModeTool;
        let result = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("exit_plan_mode"));
    }

    #[test]
    fn test_enter_plan_mode_schema() {
        let tool = EnterPlanModeTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
    }

    #[test]
    fn test_exit_plan_mode_schema() {
        let tool = ExitPlanModeTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
    }

    #[test]
    fn test_plan_mode_permissions() {
        assert_eq!(EnterPlanModeTool.permission_level(), PermissionLevel::ReadOnly);
        assert_eq!(ExitPlanModeTool.permission_level(), PermissionLevel::ReadOnly);
    }

    #[test]
    fn test_plan_mode_names() {
        assert_eq!(EnterPlanModeTool.name(), "EnterPlanMode");
        assert_eq!(ExitPlanModeTool.name(), "ExitPlanMode");
    }
}
