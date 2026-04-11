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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolContext;

    fn ctx(dir: &std::path::Path) -> ToolContext {
        ToolContext {
            cwd: dir.to_string_lossy().to_string(),
            interactive: false,
            progress_tx: None,
        }
    }

    #[tokio::test]
    async fn test_write_new_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.txt");

        let tool = WriteTool;
        let result = tool
            .call(
                serde_json::json!({
                    "file_path": path.to_string_lossy(),
                    "content": "hello world"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("11 bytes"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello world");
    }

    #[tokio::test]
    async fn test_write_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("c").join("deep.txt");

        let tool = WriteTool;
        let result = tool
            .call(
                serde_json::json!({
                    "file_path": path.to_string_lossy(),
                    "content": "deep content"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "deep content");
    }

    #[tokio::test]
    async fn test_write_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("existing.txt");
        std::fs::write(&path, "old content").unwrap();

        let tool = WriteTool;
        let result = tool
            .call(
                serde_json::json!({
                    "file_path": path.to_string_lossy(),
                    "content": "new content"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new content");
    }
}
