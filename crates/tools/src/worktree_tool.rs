//! Worktree tools — EnterWorktree and ExitWorktree.
//!
//! Matches Claude Code's git worktree isolation for agents.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

pub struct EnterWorktreeTool;

#[async_trait::async_trait]
impl Tool for EnterWorktreeTool {
    fn name(&self) -> &str {
        "EnterWorktree"
    }

    fn description(&self) -> &str {
        "Create and enter a git worktree for isolated work. The runtime handles the actual `git worktree add` operation."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "branch": {
                    "type": "string",
                    "description": "Optional branch name for the worktree"
                }
            },
            "required": []
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let branch = input["branch"].as_str().map(|s| s.to_string());

        let mut request = json!({
            "action": "enter_worktree",
        });

        if let Some(branch) = &branch {
            if branch.trim().is_empty() {
                return Ok(ToolResult::error(
                    "Branch name must be non-empty if provided".to_string(),
                ));
            }
            request["branch"] = json!(branch);
        }

        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
}

pub struct ExitWorktreeTool;

#[async_trait::async_trait]
impl Tool for ExitWorktreeTool {
    fn name(&self) -> &str {
        "ExitWorktree"
    }

    fn description(&self) -> &str {
        "Exit the current git worktree, optionally merging changes back."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "apply_changes": {
                    "type": "boolean",
                    "description": "Whether to merge changes back to the original branch (default: false)"
                }
            },
            "required": []
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let apply_changes = input["apply_changes"].as_bool().unwrap_or(false);

        let request = json!({
            "action": "exit_worktree",
            "apply_changes": apply_changes,
        });

        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
}

/// AskUserQuestion tool — prompt the user for input.
pub struct AskUserQuestionTool;

#[async_trait::async_trait]
impl Tool for AskUserQuestionTool {
    fn name(&self) -> &str {
        "AskUserQuestion"
    }

    fn description(&self) -> &str {
        "Ask the user a question and wait for their response. Use sparingly — only when genuinely stuck."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user"
                }
            },
            "required": ["question"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let question = input["question"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'question' parameter"))?;

        let request = json!({
            "action": "ask_user",
            "question": question,
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
    async fn test_enter_worktree_no_branch() {
        let tool = EnterWorktreeTool;
        let result = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("enter_worktree"));
    }

    #[tokio::test]
    async fn test_enter_worktree_with_branch() {
        let tool = EnterWorktreeTool;
        let result = tool
            .call(json!({"branch": "feature/new-thing"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("enter_worktree"));
        assert!(result.output.contains("feature/new-thing"));
    }

    #[tokio::test]
    async fn test_enter_worktree_empty_branch() {
        let tool = EnterWorktreeTool;
        let result = tool
            .call(json!({"branch": "  "}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("non-empty"));
    }

    #[tokio::test]
    async fn test_exit_worktree_default() {
        let tool = ExitWorktreeTool;
        let result = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("exit_worktree"));
        assert!(result.output.contains("false"));
    }

    #[tokio::test]
    async fn test_exit_worktree_apply_changes() {
        let tool = ExitWorktreeTool;
        let result = tool
            .call(json!({"apply_changes": true}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("exit_worktree"));
        assert!(result.output.contains("true"));
    }

    #[tokio::test]
    async fn test_ask_user_question() {
        let tool = AskUserQuestionTool;
        let result = tool
            .call(json!({"question": "Which approach do you prefer?"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("ask_user"));
        assert!(result.output.contains("Which approach"));
    }

    #[tokio::test]
    async fn test_ask_user_missing_question() {
        let tool = AskUserQuestionTool;
        let result = tool.call(json!({}), &ctx()).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_worktree_tool_names() {
        assert_eq!(EnterWorktreeTool.name(), "EnterWorktree");
        assert_eq!(ExitWorktreeTool.name(), "ExitWorktree");
        assert_eq!(AskUserQuestionTool.name(), "AskUserQuestion");
    }

    #[test]
    fn test_worktree_permissions() {
        assert_eq!(EnterWorktreeTool.permission_level(), PermissionLevel::Execute);
        assert_eq!(ExitWorktreeTool.permission_level(), PermissionLevel::Execute);
        assert_eq!(AskUserQuestionTool.permission_level(), PermissionLevel::ReadOnly);
    }
}
