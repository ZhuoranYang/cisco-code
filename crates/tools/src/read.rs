//! Read tool — read files with line numbers and offset/limit.
//!
//! Pattern from Claude Code's FileReadTool: cat -n format output,
//! offset/limit for large files, BOM stripping.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;
use std::path::Path;

use crate::{Tool, ToolContext};

pub struct ReadTool;

impl Tool for ReadTool {
    fn name(&self) -> &str {
        "Read"
    }

    fn description(&self) -> &str {
        "Reads a file from the local filesystem. Returns content with line numbers. Use offset and limit for large files."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (0-indexed)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Number of lines to read (default: 2000)"
                }
            },
            "required": ["file_path"]
        })
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult> {
        let file_path = input["file_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'file_path'"))?;

        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().unwrap_or(2000) as usize;

        // Resolve relative paths against cwd
        let path = if Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            Path::new(&ctx.cwd)
                .join(file_path)
                .to_string_lossy()
                .to_string()
        };

        // Read file
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::error(format!("Failed to read {path}: {e}"))),
        };

        // Strip BOM
        let content = if content.starts_with('\u{FEFF}') {
            &content[3..]
        } else {
            &content
        };

        // Apply offset and limit, format with line numbers (cat -n style)
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();
        let end = (offset + limit).min(total_lines);
        let selected = &lines[offset.min(total_lines)..end];

        let mut output = String::new();
        for (i, line) in selected.iter().enumerate() {
            let line_num = offset + i + 1; // 1-indexed
            output.push_str(&format!("{line_num}\t{line}\n"));
        }

        if output.is_empty() {
            output = "(empty file)".to_string();
        }

        if end < total_lines {
            output.push_str(&format!(
                "\n... ({} more lines, use offset to read more)",
                total_lines - end
            ));
        }

        Ok(ToolResult::success(output))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
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
        }
    }

    #[tokio::test]
    async fn test_read_basic_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hello.txt");
        std::fs::write(&path, "line one\nline two\nline three\n").unwrap();

        let tool = ReadTool;
        let result = tool
            .call(
                serde_json::json!({"file_path": path.to_string_lossy()}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("1\tline one"));
        assert!(result.output.contains("2\tline two"));
        assert!(result.output.contains("3\tline three"));
    }

    #[tokio::test]
    async fn test_read_with_offset_and_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lines.txt");
        let content: String = (1..=100).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&path, &content).unwrap();

        let tool = ReadTool;
        let result = tool
            .call(
                serde_json::json!({
                    "file_path": path.to_string_lossy(),
                    "offset": 10,
                    "limit": 5
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("11\tline 11"));
        assert!(result.output.contains("15\tline 15"));
        assert!(!result.output.contains("16\tline 16"));
        assert!(result.output.contains("more lines"));
    }

    #[tokio::test]
    async fn test_read_nonexistent_file() {
        let tool = ReadTool;
        let ctx = ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };
        let result = tool
            .call(
                serde_json::json!({"file_path": "/tmp/does_not_exist_xyz.txt"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("Failed to read"));
    }

    #[tokio::test]
    async fn test_read_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        std::fs::write(&path, "").unwrap();

        let tool = ReadTool;
        let result = tool
            .call(
                serde_json::json!({"file_path": path.to_string_lossy()}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("empty"));
    }

    #[tokio::test]
    async fn test_read_bom_stripped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bom.txt");
        let mut content = vec![0xEF, 0xBB, 0xBF]; // UTF-8 BOM
        content.extend_from_slice(b"hello");
        std::fs::write(&path, &content).unwrap();

        let tool = ReadTool;
        let result = tool
            .call(
                serde_json::json!({"file_path": path.to_string_lossy()}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("hello"));
    }
}
