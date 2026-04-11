//! Skill tool — invoke registered skills/slash-commands.
//!
//! Matches Claude Code's Skill tool: executes user-invocable skills
//! like /commit, /review-pr, /pdf, etc.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

pub struct SkillTool;

impl Tool for SkillTool {
    fn name(&self) -> &str {
        "Skill"
    }

    fn description(&self) -> &str {
        "Execute a skill (slash command) within the current conversation. Skills provide specialized capabilities like committing code, reviewing PRs, etc."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "The skill name (e.g., 'commit', 'review-pr', 'pdf')"
                },
                "args": {
                    "type": "string",
                    "description": "Optional arguments for the skill"
                }
            },
            "required": ["skill"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let skill = input["skill"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'skill' parameter"))?;

        if skill.trim().is_empty() {
            return Ok(ToolResult::error("Skill name cannot be empty".to_string()));
        }

        let args = input["args"].as_str();

        // Build the skill invocation request
        let request = json!({
            "action": "invoke_skill",
            "skill": skill.trim(),
            "args": args,
        });

        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
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
    async fn test_skill_basic() {
        let tool = SkillTool;
        let result = tool
            .call(json!({"skill": "commit"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("invoke_skill"));
        assert!(result.output.contains("commit"));
    }

    #[tokio::test]
    async fn test_skill_with_args() {
        let tool = SkillTool;
        let result = tool
            .call(json!({"skill": "commit", "args": "-m 'Fix bug'"}), &ctx())
            .await
            .unwrap();
        assert!(result.output.contains("Fix bug"));
    }

    #[tokio::test]
    async fn test_skill_empty_name() {
        let tool = SkillTool;
        let result = tool
            .call(json!({"skill": "  "}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_skill_missing_name() {
        let tool = SkillTool;
        let result = tool.call(json!({}), &ctx()).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_skill_schema() {
        let tool = SkillTool;
        let schema = tool.input_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("skill")));
    }

    #[test]
    fn test_skill_permission() {
        let tool = SkillTool;
        assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    }
}
