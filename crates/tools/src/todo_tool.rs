//! TodoWrite tool — structured todo management.
//!
//! Persists a todo list to `.cisco-code/todos.json` in the project root.
//! The prompt builder reads this file and injects active todos into the
//! dynamic section of the system prompt.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

use crate::{Tool, ToolContext};

pub struct TodoWriteTool;

/// A single todo item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: TodoStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
}

/// Status of a todo item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Done,
}

impl std::fmt::Display for TodoStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TodoStatus::Pending => write!(f, "pending"),
            TodoStatus::InProgress => write!(f, "in_progress"),
            TodoStatus::Done => write!(f, "done"),
        }
    }
}

impl Tool for TodoWriteTool {
    fn name(&self) -> &str {
        "TodoWrite"
    }

    fn description(&self) -> &str {
        "Write the complete todo list. Replaces the entire list with the provided items. Persisted to .cisco-code/todos.json."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "todos": {
                    "type": "array",
                    "description": "Complete todo list (replaces existing)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "Unique identifier for the todo"
                            },
                            "content": {
                                "type": "string",
                                "description": "Description of the todo item"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "done"],
                                "description": "Current status"
                            },
                            "priority": {
                                "type": "integer",
                                "description": "Priority (1=highest, optional)"
                            }
                        },
                        "required": ["id", "content", "status"]
                    }
                }
            },
            "required": ["todos"]
        })
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult> {
        let todos_value = input.get("todos")
            .ok_or_else(|| anyhow::anyhow!("missing 'todos'"))?;

        let todos: Vec<TodoItem> = match serde_json::from_value(todos_value.clone()) {
            Ok(t) => t,
            Err(e) => return Ok(ToolResult::error(format!("Invalid todo format: {e}"))),
        };

        // Validate no duplicate IDs
        let mut ids = std::collections::HashSet::new();
        for todo in &todos {
            if !ids.insert(&todo.id) {
                return Ok(ToolResult::error(format!(
                    "Duplicate todo ID: '{}'", todo.id
                )));
            }
        }

        // Write to .cisco-code/todos.json
        let dir = Path::new(&ctx.cwd).join(".cisco-code");
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            return Ok(ToolResult::error(format!(
                "Failed to create .cisco-code directory: {e}"
            )));
        }

        let file_path = dir.join("todos.json");
        let content = serde_json::to_string_pretty(&todos)?;

        match tokio::fs::write(&file_path, &content).await {
            Ok(()) => {
                let pending = todos.iter().filter(|t| t.status == TodoStatus::Pending).count();
                let in_progress = todos.iter().filter(|t| t.status == TodoStatus::InProgress).count();
                let done = todos.iter().filter(|t| t.status == TodoStatus::Done).count();

                Ok(ToolResult::success(format!(
                    "Wrote {} todo(s) to {}: {} pending, {} in progress, {} done",
                    todos.len(),
                    file_path.display(),
                    pending,
                    in_progress,
                    done,
                )))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to write todos: {e}"
            ))),
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::WorkspaceWrite
    }
}

/// Read the current todo list from .cisco-code/todos.json.
pub fn read_todos(cwd: &str) -> Result<Vec<TodoItem>> {
    let file_path = Path::new(cwd).join(".cisco-code/todos.json");
    if !file_path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&file_path)?;
    let todos: Vec<TodoItem> = serde_json::from_str(&content)?;
    Ok(todos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_todo_item_serialization() {
        let item = TodoItem {
            id: "1".into(),
            content: "Fix bug".into(),
            status: TodoStatus::Pending,
            priority: Some(1),
        };
        let json = serde_json::to_string(&item).unwrap();
        assert!(json.contains("\"pending\""));
        assert!(json.contains("\"priority\":1"));
    }

    #[test]
    fn test_todo_item_deserialization() {
        let json = r#"{"id":"2","content":"Add tests","status":"in_progress"}"#;
        let item: TodoItem = serde_json::from_str(json).unwrap();
        assert_eq!(item.id, "2");
        assert_eq!(item.status, TodoStatus::InProgress);
        assert!(item.priority.is_none());
    }

    #[tokio::test]
    async fn test_todo_write_basic() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TodoWriteTool;
        let ctx = crate::ToolContext {
            cwd: dir.path().to_string_lossy().to_string(),
            interactive: false,
            progress_tx: None,
        };

        let input = json!({
            "todos": [
                {"id": "1", "content": "Fix bug", "status": "pending", "priority": 1},
                {"id": "2", "content": "Add tests", "status": "in_progress"},
                {"id": "3", "content": "Deploy", "status": "done"}
            ]
        });

        let result = tool.call(input, &ctx).await.unwrap();
        assert!(!result.is_error, "Error: {}", result.output);
        assert!(result.output.contains("3 todo(s)"));
        assert!(result.output.contains("1 pending"));
        assert!(result.output.contains("1 in progress"));
        assert!(result.output.contains("1 done"));

        // Verify file was written
        let file_path = dir.path().join(".cisco-code/todos.json");
        assert!(file_path.exists());
        let content = std::fs::read_to_string(&file_path).unwrap();
        let todos: Vec<TodoItem> = serde_json::from_str(&content).unwrap();
        assert_eq!(todos.len(), 3);
    }

    #[tokio::test]
    async fn test_todo_write_duplicate_ids() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TodoWriteTool;
        let ctx = crate::ToolContext {
            cwd: dir.path().to_string_lossy().to_string(),
            interactive: false,
            progress_tx: None,
        };

        let input = json!({
            "todos": [
                {"id": "1", "content": "First", "status": "pending"},
                {"id": "1", "content": "Duplicate", "status": "done"}
            ]
        });

        let result = tool.call(input, &ctx).await.unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("Duplicate"));
    }

    #[tokio::test]
    async fn test_todo_write_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let tool = TodoWriteTool;
        let ctx = crate::ToolContext {
            cwd: dir.path().to_string_lossy().to_string(),
            interactive: false,
            progress_tx: None,
        };

        let input = json!({"todos": []});
        let result = tool.call(input, &ctx).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("0 todo(s)"));
    }

    #[test]
    fn test_read_todos_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let todos = read_todos(&dir.path().to_string_lossy()).unwrap();
        assert!(todos.is_empty());
    }

    #[test]
    fn test_read_todos_existing() {
        let dir = tempfile::tempdir().unwrap();
        let cisco_dir = dir.path().join(".cisco-code");
        std::fs::create_dir_all(&cisco_dir).unwrap();
        std::fs::write(
            cisco_dir.join("todos.json"),
            r#"[{"id":"1","content":"test","status":"pending"}]"#,
        ).unwrap();

        let todos = read_todos(&dir.path().to_string_lossy()).unwrap();
        assert_eq!(todos.len(), 1);
        assert_eq!(todos[0].content, "test");
    }
}
