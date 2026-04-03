//! Write tool — create or overwrite files.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;
use std::path::Path;

use crate::{Tool, ToolContext};

pub struct WriteTool;

impl Tool for WriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn description(&self) -> &str {
        "Writes content to a file, creating it if it doesn't exist or overwriting if it does. Creates parent directories as needed."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult> {
        let file_path = input["file_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'file_path'"))?;
        let content = input["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'content'"))?;

        let path = if Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            Path::new(&ctx.cwd)
                .join(file_path)
                .to_string_lossy()
                .to_string()
        };

        // Create parent directories
        if let Some(parent) = Path::new(&path).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        match tokio::fs::write(&path, content).await {
            Ok(()) => Ok(ToolResult::success(format!(
                "Successfully wrote {} bytes to {path}",
                content.len()
            ))),
            Err(e) => Ok(ToolResult::error(format!("Failed to write {path}: {e}"))),
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::WorkspaceWrite
    }
}
