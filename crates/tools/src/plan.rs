//! Plan mode tools — EnterPlanMode and ExitPlanMode.
//!
//! Matches Claude Code v2.1.88's plan mode architecture:
//! - EnterPlanMode: switches permission mode to Plan (read-only)
//! - ExitPlanMode: writes plan to disk, restores previous permission mode
//!
//! These tools emit JSON action descriptors that the ConversationRuntime
//! intercepts to manage plan mode state transitions. The runtime handles
//! the actual permission mode changes and plan file I/O via PlanManager.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

/// Enter plan mode — switch to planning/design focus.
///
/// Matches Claude Code's EnterPlanModeTool:
/// - Empty input schema (no parameters needed)
/// - Sets permission mode to 'plan' (read-only)
/// - Returns instructions for the 5-phase planning workflow
pub struct EnterPlanModeTool;

#[async_trait::async_trait]
impl Tool for EnterPlanModeTool {
    fn name(&self) -> &str {
        "EnterPlanMode"
    }

    fn description(&self) -> &str {
        "Switch to plan mode to design an approach before coding. In plan mode, \
         you focus on research, analysis, and planning — no code changes are made. \
         Only read-only tools (Read, Grep, Glob, WebSearch, WebFetch, Agent) are permitted."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn call(&self, _input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        // Emit an action descriptor for the runtime to intercept.
        // The runtime handles the actual permission mode transition.
        let action = json!({
            "action": "enter_plan_mode",
            "message": "Entered plan mode. You are now in read-only planning mode."
        });

        Ok(ToolResult::success(format!(
            "{}\n\n{}",
            serde_json::to_string_pretty(&action)?,
            PLAN_MODE_INSTRUCTIONS
        )))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }
}

/// Exit plan mode — present plan for approval and start coding.
///
/// Matches Claude Code's ExitPlanModeV2Tool:
/// - Reads plan from disk (written during plan mode)
/// - Restores previous permission mode
/// - Returns plan content for user approval
pub struct ExitPlanModeTool;

#[async_trait::async_trait]
impl Tool for ExitPlanModeTool {
    fn name(&self) -> &str {
        "ExitPlanMode"
    }

    fn description(&self) -> &str {
        "Present your implementation plan for approval and exit plan mode. \
         This restores the previous permission mode so you can start coding. \
         Call this after you've designed a thorough plan."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "string",
                    "description": "The implementation plan in markdown format. This will be saved to disk."
                }
            },
            "required": []
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let plan = input["plan"].as_str().map(|s| s.to_string());

        // Emit an action descriptor for the runtime to intercept.
        // The runtime handles: saving plan to disk, restoring permission mode,
        // firing PlanModeExit hook.
        let mut action = json!({
            "action": "exit_plan_mode",
        });

        if let Some(ref plan_content) = plan {
            action["plan"] = json!(plan_content);
        }

        let message = if let Some(ref plan_content) = plan {
            format!(
                "{}\n\nPlan approved. The following plan has been saved:\n\n{}",
                serde_json::to_string_pretty(&action)?,
                plan_content
            )
        } else {
            format!(
                "{}\n\nPlan mode exited. You can proceed with implementation.",
                serde_json::to_string_pretty(&action)?
            )
        };

        Ok(ToolResult::success(message))
    }

    fn permission_level(&self) -> PermissionLevel {
        // ExitPlanMode needs to write the plan file, but the runtime handles
        // the actual write. The tool itself is read-only from the permission
        // engine's perspective.
        PermissionLevel::ReadOnly
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }
}

/// Instructions displayed when entering plan mode.
///
/// Matches Claude Code's 5-phase planning workflow:
const PLAN_MODE_INSTRUCTIONS: &str = "\
DO NOT write or edit any files except the plan file. Follow this workflow:

1. **Explore**: Thoroughly research the codebase to understand the relevant code, \
   architecture, and patterns. Use Read, Grep, Glob, and Agent tools.

2. **Identify patterns**: Find similar features or patterns in the codebase that \
   you can follow. Understanding existing conventions is critical.

3. **Consider approaches**: Think through multiple implementation approaches. \
   Consider trade-offs, risks, and complexity.

4. **Clarify**: If anything is ambiguous, use AskUserQuestion to get clarification \
   before finalizing the plan.

5. **Design**: Create a concrete, step-by-step implementation strategy. Include \
   specific file paths, function names, and code snippets where helpful. \
   When ready, call ExitPlanMode with your plan to present it for approval.";

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
        let result = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("enter_plan_mode"));
        assert!(result.output.contains("Explore"));
        assert!(result.output.contains("ExitPlanMode"));
    }

    #[tokio::test]
    async fn test_enter_plan_mode_ignores_extra_input() {
        let tool = EnterPlanModeTool;
        let result = tool
            .call(json!({"extra": "ignored"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("enter_plan_mode"));
    }

    #[tokio::test]
    async fn test_exit_plan_mode_with_plan() {
        let tool = ExitPlanModeTool;
        let plan = "## Plan\n\n1. Refactor auth module\n2. Add tests";
        let result = tool
            .call(json!({"plan": plan}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("exit_plan_mode"));
        assert!(result.output.contains("Refactor auth module"));
        assert!(result.output.contains("Plan approved"));
    }

    #[tokio::test]
    async fn test_exit_plan_mode_without_plan() {
        let tool = ExitPlanModeTool;
        let result = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("exit_plan_mode"));
        assert!(result.output.contains("proceed with implementation"));
    }

    #[test]
    fn test_enter_plan_mode_schema() {
        let tool = EnterPlanModeTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        // Empty properties — no parameters needed
        let props = schema["properties"].as_object().unwrap();
        assert!(props.is_empty());
    }

    #[test]
    fn test_exit_plan_mode_schema() {
        let tool = ExitPlanModeTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].get("plan").is_some());
    }

    #[test]
    fn test_plan_mode_permissions() {
        assert_eq!(EnterPlanModeTool.permission_level(), PermissionLevel::ReadOnly);
        assert_eq!(ExitPlanModeTool.permission_level(), PermissionLevel::ReadOnly);
    }

    #[test]
    fn test_plan_mode_concurrency_safe() {
        assert!(EnterPlanModeTool.is_concurrency_safe());
        assert!(ExitPlanModeTool.is_concurrency_safe());
    }

    #[test]
    fn test_plan_mode_names() {
        assert_eq!(EnterPlanModeTool.name(), "EnterPlanMode");
        assert_eq!(ExitPlanModeTool.name(), "ExitPlanMode");
    }

    #[test]
    fn test_plan_mode_instructions_content() {
        assert!(PLAN_MODE_INSTRUCTIONS.contains("Explore"));
        assert!(PLAN_MODE_INSTRUCTIONS.contains("patterns"));
        assert!(PLAN_MODE_INSTRUCTIONS.contains("approaches"));
        assert!(PLAN_MODE_INSTRUCTIONS.contains("Clarify"));
        assert!(PLAN_MODE_INSTRUCTIONS.contains("Design"));
    }
}
