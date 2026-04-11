//! Task tools — TaskCreate, TaskUpdate, TaskList, TaskGet, TaskOutput, TaskStop.
//!
//! Matches Claude Code's task management tools for structured work tracking.
//! These tools package requests; the runtime TaskManager handles state.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

pub struct TaskCreateTool;

impl Tool for TaskCreateTool {
    fn name(&self) -> &str {
        "TaskCreate"
    }

    fn description(&self) -> &str {
        "Create a new task to track work progress."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "description": {
                    "type": "string",
                    "description": "Description of the task"
                }
            },
            "required": ["description"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let description = input["description"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'description' parameter"))?;

        let request = json!({
            "action": "task_create",
            "description": description,
        });

        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
}

pub struct TaskUpdateTool;

impl Tool for TaskUpdateTool {
    fn name(&self) -> &str {
        "TaskUpdate"
    }

    fn description(&self) -> &str {
        "Update task status (pending, in_progress, completed, cancelled, blocked)."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "integer",
                    "description": "Task ID"
                },
                "status": {
                    "type": "string",
                    "description": "New status",
                    "enum": ["pending", "in_progress", "completed", "cancelled", "blocked"]
                },
                "output": {
                    "type": "string",
                    "description": "Optional output/notes"
                }
            },
            "required": ["id", "status"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let id = input["id"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;

        let status = input["status"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'status' parameter"))?;

        let valid_statuses = ["pending", "in_progress", "completed", "cancelled", "blocked"];
        if !valid_statuses.contains(&status) {
            return Ok(ToolResult::error(format!(
                "Invalid status: {}. Use one of: {}",
                status,
                valid_statuses.join(", ")
            )));
        }

        let request = json!({
            "action": "task_update",
            "id": id,
            "status": status,
            "output": input["output"].as_str(),
        });

        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
}

pub struct TaskListTool;

impl Tool for TaskListTool {
    fn name(&self) -> &str {
        "TaskList"
    }

    fn description(&self) -> &str {
        "List all tasks, optionally filtered by status."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "description": "Filter by status (optional)",
                    "enum": ["pending", "in_progress", "completed", "cancelled", "blocked"]
                }
            },
            "required": []
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let request = json!({
            "action": "task_list",
            "status": input["status"].as_str(),
        });

        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }
}

pub struct TaskGetTool;

impl Tool for TaskGetTool {
    fn name(&self) -> &str {
        "TaskGet"
    }

    fn description(&self) -> &str {
        "Get details of a specific task by ID."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "integer",
                    "description": "Task ID"
                }
            },
            "required": ["id"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let id = input["id"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;

        let request = json!({
            "action": "task_get",
            "id": id,
        });

        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }
}

pub struct TaskOutputTool;

impl Tool for TaskOutputTool {
    fn name(&self) -> &str {
        "TaskOutput"
    }

    fn description(&self) -> &str {
        "Read the output of a completed task."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "integer",
                    "description": "Task ID"
                }
            },
            "required": ["id"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let id = input["id"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;

        let request = json!({
            "action": "task_output",
            "id": id,
        });

        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }
}

pub struct TaskStopTool;

impl Tool for TaskStopTool {
    fn name(&self) -> &str {
        "TaskStop"
    }

    fn description(&self) -> &str {
        "Stop/cancel a running task."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "integer",
                    "description": "Task ID"
                }
            },
            "required": ["id"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let id = input["id"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("missing 'id' parameter"))?;

        let request = json!({
            "action": "task_stop",
            "id": id,
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
    async fn test_task_create() {
        let tool = TaskCreateTool;
        let result = tool
            .call(json!({"description": "Implement feature X"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("task_create"));
        assert!(result.output.contains("Implement feature X"));
    }

    #[tokio::test]
    async fn test_task_create_missing_desc() {
        let tool = TaskCreateTool;
        let result = tool.call(json!({}), &ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_task_update() {
        let tool = TaskUpdateTool;
        let result = tool
            .call(json!({"id": 1, "status": "completed", "output": "Done"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("task_update"));
        assert!(result.output.contains("completed"));
    }

    #[tokio::test]
    async fn test_task_update_invalid_status() {
        let tool = TaskUpdateTool;
        let result = tool
            .call(json!({"id": 1, "status": "invalid"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_task_list() {
        let tool = TaskListTool;
        let result = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("task_list"));
    }

    #[tokio::test]
    async fn test_task_list_with_filter() {
        let tool = TaskListTool;
        let result = tool
            .call(json!({"status": "in_progress"}), &ctx())
            .await
            .unwrap();
        assert!(result.output.contains("in_progress"));
    }

    #[tokio::test]
    async fn test_task_get() {
        let tool = TaskGetTool;
        let result = tool
            .call(json!({"id": 42}), &ctx())
            .await
            .unwrap();
        assert!(result.output.contains("task_get"));
        assert!(result.output.contains("42"));
    }

    #[tokio::test]
    async fn test_task_output() {
        let tool = TaskOutputTool;
        let result = tool
            .call(json!({"id": 1}), &ctx())
            .await
            .unwrap();
        assert!(result.output.contains("task_output"));
    }

    #[tokio::test]
    async fn test_task_stop() {
        let tool = TaskStopTool;
        let result = tool
            .call(json!({"id": 5}), &ctx())
            .await
            .unwrap();
        assert!(result.output.contains("task_stop"));
    }

    #[test]
    fn test_task_tool_names() {
        assert_eq!(TaskCreateTool.name(), "TaskCreate");
        assert_eq!(TaskUpdateTool.name(), "TaskUpdate");
        assert_eq!(TaskListTool.name(), "TaskList");
        assert_eq!(TaskGetTool.name(), "TaskGet");
        assert_eq!(TaskOutputTool.name(), "TaskOutput");
        assert_eq!(TaskStopTool.name(), "TaskStop");
    }

    #[test]
    fn test_task_permissions() {
        assert_eq!(TaskCreateTool.permission_level(), PermissionLevel::ReadOnly);
        assert_eq!(TaskUpdateTool.permission_level(), PermissionLevel::ReadOnly);
    }
}
