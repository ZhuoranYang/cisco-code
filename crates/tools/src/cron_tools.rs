//! Cron tools — CronCreate, CronList, CronDelete.
//!
//! Matches Claude Code's cron scheduling tools for recurring prompts.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

pub struct CronCreateTool;

impl Tool for CronCreateTool {
    fn name(&self) -> &str {
        "CronCreate"
    }

    fn description(&self) -> &str {
        "Create a scheduled cron job that runs a prompt on a schedule."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name for the cron job"
                },
                "prompt": {
                    "type": "string",
                    "description": "The prompt to execute on schedule"
                },
                "schedule": {
                    "type": "string",
                    "description": "Schedule: 'once:<ISO datetime>', 'interval:<seconds>', or cron expression"
                }
            },
            "required": ["name", "prompt", "schedule"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let name = input["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'name' parameter"))?;
        let prompt = input["prompt"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'prompt' parameter"))?;
        let schedule = input["schedule"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'schedule' parameter"))?;

        let request = json!({
            "action": "cron_create",
            "name": name,
            "prompt": prompt,
            "schedule": schedule,
        });

        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
}

pub struct CronListTool;

impl Tool for CronListTool {
    fn name(&self) -> &str {
        "CronList"
    }

    fn description(&self) -> &str {
        "List all scheduled cron jobs."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn call(&self, _input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let request = json!({ "action": "cron_list" });
        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }
}

pub struct CronDeleteTool;

impl Tool for CronDeleteTool {
    fn name(&self) -> &str {
        "CronDelete"
    }

    fn description(&self) -> &str {
        "Delete a scheduled cron job by ID."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Cron job ID to delete"
                }
            },
            "required": ["id"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let id = input["id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;

        let request = json!({
            "action": "cron_delete",
            "id": id,
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
    async fn test_cron_create() {
        let tool = CronCreateTool;
        let result = tool
            .call(
                json!({
                    "name": "daily-test",
                    "prompt": "Run tests",
                    "schedule": "interval:86400"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("cron_create"));
    }

    #[tokio::test]
    async fn test_cron_create_missing_fields() {
        let tool = CronCreateTool;
        let result = tool.call(json!({"name": "test"}), &ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cron_list() {
        let tool = CronListTool;
        let result = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("cron_list"));
    }

    #[tokio::test]
    async fn test_cron_delete() {
        let tool = CronDeleteTool;
        let result = tool
            .call(json!({"id": "cron-1"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("cron_delete"));
    }

    #[tokio::test]
    async fn test_cron_delete_missing_id() {
        let tool = CronDeleteTool;
        let result = tool.call(json!({}), &ctx()).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_cron_tool_names() {
        assert_eq!(CronCreateTool.name(), "CronCreate");
        assert_eq!(CronListTool.name(), "CronList");
        assert_eq!(CronDeleteTool.name(), "CronDelete");
    }

    #[test]
    fn test_cron_permissions() {
        assert_eq!(CronCreateTool.permission_level(), PermissionLevel::Execute);
        assert_eq!(CronListTool.permission_level(), PermissionLevel::ReadOnly);
        assert_eq!(CronDeleteTool.permission_level(), PermissionLevel::Execute);
    }
}
