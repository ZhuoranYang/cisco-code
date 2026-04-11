//! LSP tool — interact with Language Server Protocol servers.
//!
//! Matches Claude Code's LSP tool: supports goToDefinition, findReferences,
//! hover, documentSymbol, workspaceSymbol, and diagnostics actions.
//! The actual LSP client lives in the runtime; this tool validates
//! parameters and packages LSP requests.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

/// Valid LSP actions.
const VALID_ACTIONS: &[&str] = &[
    "goToDefinition",
    "findReferences",
    "hover",
    "documentSymbol",
    "workspaceSymbol",
    "diagnostics",
];

/// Actions that require position (line + character).
const POSITION_ACTIONS: &[&str] = &["goToDefinition", "findReferences", "hover"];

pub struct LspTool;

impl Tool for LspTool {
    fn name(&self) -> &str {
        "LSP"
    }

    fn description(&self) -> &str {
        "Interact with language servers for code intelligence. Supports goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, and diagnostics."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "LSP action to perform",
                    "enum": VALID_ACTIONS
                },
                "file_path": {
                    "type": "string",
                    "description": "File path (required for most actions)"
                },
                "line": {
                    "type": "integer",
                    "description": "Line number (1-indexed, required for position actions)"
                },
                "character": {
                    "type": "integer",
                    "description": "Character offset (0-indexed, required for position actions)"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (required for workspaceSymbol)"
                }
            },
            "required": ["action"]
        })
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult> {
        let action = input["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'action' parameter"))?;

        if !VALID_ACTIONS.contains(&action) {
            return Ok(ToolResult::error(format!(
                "Unknown action: {}. Valid actions: {}",
                action,
                VALID_ACTIONS.join(", ")
            )));
        }

        // Validate required parameters per action
        if action == "workspaceSymbol" {
            if input["query"].as_str().is_none() {
                return Ok(ToolResult::error(
                    "workspaceSymbol requires 'query' parameter".to_string(),
                ));
            }
        } else {
            // All other actions require file_path
            if input["file_path"].as_str().is_none() {
                return Ok(ToolResult::error(format!(
                    "{} requires 'file_path' parameter",
                    action
                )));
            }
        }

        if POSITION_ACTIONS.contains(&action) {
            if input["line"].as_u64().is_none() {
                return Ok(ToolResult::error(format!(
                    "{} requires 'line' parameter",
                    action
                )));
            }
            if input["character"].as_u64().is_none() {
                return Ok(ToolResult::error(format!(
                    "{} requires 'character' parameter",
                    action
                )));
            }
        }

        // Build LSP request descriptor
        let mut request = json!({
            "action": "lsp_request",
            "method": action,
        });

        if let Some(file_path) = input["file_path"].as_str() {
            let path = if std::path::Path::new(file_path).is_absolute() {
                file_path.to_string()
            } else {
                std::path::PathBuf::from(&ctx.cwd)
                    .join(file_path)
                    .to_string_lossy()
                    .to_string()
            };
            request["file_path"] = json!(path);
        }

        if let Some(line) = input["line"].as_u64() {
            request["line"] = json!(line);
        }
        if let Some(character) = input["character"].as_u64() {
            request["character"] = json!(character);
        }
        if let Some(query) = input["query"].as_str() {
            request["query"] = json!(query);
        }

        Ok(ToolResult::success(serde_json::to_string_pretty(&request)?))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn is_concurrency_safe(&self) -> bool {
        true
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
    async fn test_lsp_go_to_definition() {
        let tool = LspTool;
        let result = tool
            .call(
                json!({
                    "action": "goToDefinition",
                    "file_path": "src/main.rs",
                    "line": 10,
                    "character": 5
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("goToDefinition"));
    }

    #[tokio::test]
    async fn test_lsp_workspace_symbol() {
        let tool = LspTool;
        let result = tool
            .call(
                json!({
                    "action": "workspaceSymbol",
                    "query": "MyStruct"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("MyStruct"));
    }

    #[tokio::test]
    async fn test_lsp_invalid_action() {
        let tool = LspTool;
        let result = tool
            .call(json!({"action": "invalid"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_lsp_missing_file_path() {
        let tool = LspTool;
        let result = tool
            .call(json!({"action": "documentSymbol"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("file_path"));
    }

    #[tokio::test]
    async fn test_lsp_missing_position() {
        let tool = LspTool;
        let result = tool
            .call(
                json!({
                    "action": "hover",
                    "file_path": "test.rs"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("line"));
    }

    #[tokio::test]
    async fn test_lsp_workspace_symbol_missing_query() {
        let tool = LspTool;
        let result = tool
            .call(json!({"action": "workspaceSymbol"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("query"));
    }

    #[test]
    fn test_lsp_schema() {
        let tool = LspTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("action")));
    }

    #[test]
    fn test_lsp_permission() {
        let tool = LspTool;
        assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
    }

    #[tokio::test]
    async fn test_lsp_find_references() {
        let tool = LspTool;
        let result = tool
            .call(
                json!({
                    "action": "findReferences",
                    "file_path": "/tmp/main.rs",
                    "line": 42,
                    "character": 10
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("findReferences"));
    }

    #[tokio::test]
    async fn test_lsp_hover() {
        let tool = LspTool;
        let result = tool
            .call(
                json!({
                    "action": "hover",
                    "file_path": "lib.rs",
                    "line": 1,
                    "character": 0
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("hover"));
    }

    #[tokio::test]
    async fn test_lsp_document_symbol() {
        let tool = LspTool;
        let result = tool
            .call(
                json!({
                    "action": "documentSymbol",
                    "file_path": "main.rs"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("documentSymbol"));
    }

    #[tokio::test]
    async fn test_lsp_diagnostics() {
        let tool = LspTool;
        let result = tool
            .call(
                json!({
                    "action": "diagnostics",
                    "file_path": "src/lib.rs"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("diagnostics"));
    }

    #[tokio::test]
    async fn test_lsp_relative_path_resolved() {
        let tool = LspTool;
        let result = tool
            .call(
                json!({
                    "action": "documentSymbol",
                    "file_path": "src/main.rs"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        // Should resolve relative path against cwd
        assert!(result.output.contains("/tmp/src/main.rs"));
    }

    #[tokio::test]
    async fn test_lsp_absolute_path_preserved() {
        let tool = LspTool;
        let result = tool
            .call(
                json!({
                    "action": "documentSymbol",
                    "file_path": "/home/user/project/src/main.rs"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.output.contains("/home/user/project/src/main.rs"));
    }

    #[tokio::test]
    async fn test_lsp_missing_character_for_position_action() {
        let tool = LspTool;
        let result = tool
            .call(
                json!({
                    "action": "goToDefinition",
                    "file_path": "test.rs",
                    "line": 5
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("character"));
    }

    #[tokio::test]
    async fn test_lsp_missing_action() {
        let tool = LspTool;
        let result = tool.call(json!({}), &ctx()).await;
        assert!(result.is_err());
    }
}
