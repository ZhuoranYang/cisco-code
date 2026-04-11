//! NotebookEdit tool — modify Jupyter notebook (.ipynb) cells.
//!
//! Reads a .ipynb JSON file, modifies a specific cell by index,
//! and writes the result back. Supports replace, insert_before,
//! insert_after, and delete modes.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;
use std::path::Path;

use crate::{Tool, ToolContext};

pub struct NotebookEditTool;

const VALID_CELL_TYPES: &[&str] = &["code", "markdown"];
const VALID_MODES: &[&str] = &["replace", "insert_before", "insert_after", "delete"];

/// Convert a string into the notebook source format (array of lines with newlines).
fn string_to_source(s: &str) -> Vec<serde_json::Value> {
    if s.is_empty() {
        return vec![json!("")];
    }
    let lines: Vec<&str> = s.split('\n').collect();
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            if i < lines.len() - 1 {
                // All lines except the last get a trailing newline
                json!(format!("{line}\n"))
            } else {
                // Last line: no trailing newline
                json!(line.to_string())
            }
        })
        .collect()
}

/// Create a new cell with the given type and source.
fn make_cell(cell_type: &str, source: &str) -> serde_json::Value {
    let source_lines = string_to_source(source);
    match cell_type {
        "markdown" => json!({
            "cell_type": "markdown",
            "metadata": {},
            "source": source_lines
        }),
        _ => json!({
            "cell_type": "code",
            "execution_count": null,
            "metadata": {},
            "outputs": [],
            "source": source_lines
        }),
    }
}

impl Tool for NotebookEditTool {
    fn name(&self) -> &str {
        "NotebookEdit"
    }

    fn description(&self) -> &str {
        "Modify Jupyter notebook (.ipynb) cells. Supports replacing cell content, inserting new cells before/after, and deleting cells. Works with both code and markdown cells."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "notebook_path": {
                    "type": "string",
                    "description": "Path to the .ipynb notebook file"
                },
                "cell_index": {
                    "type": "integer",
                    "description": "0-based index of the cell to modify"
                },
                "new_source": {
                    "type": "string",
                    "description": "New content for the cell"
                },
                "cell_type": {
                    "type": "string",
                    "description": "Cell type: 'code' or 'markdown'. Default preserves existing type.",
                    "enum": ["code", "markdown"]
                },
                "mode": {
                    "type": "string",
                    "description": "Edit mode: 'replace' (default), 'insert_before', 'insert_after', 'delete'",
                    "enum": ["replace", "insert_before", "insert_after", "delete"]
                }
            },
            "required": ["notebook_path", "cell_index", "new_source"]
        })
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult> {
        let notebook_path = match input["notebook_path"].as_str() {
            Some(p) if !p.trim().is_empty() => p,
            Some(_) => return Ok(ToolResult::error("'notebook_path' must not be empty")),
            None => {
                return Ok(ToolResult::error(
                    "missing required parameter 'notebook_path'",
                ))
            }
        };

        let cell_index = match input["cell_index"].as_u64() {
            Some(i) => i as usize,
            None => {
                return Ok(ToolResult::error(
                    "missing or invalid 'cell_index' (must be a non-negative integer)",
                ))
            }
        };

        let new_source = match input["new_source"].as_str() {
            Some(s) => s,
            None => {
                return Ok(ToolResult::error(
                    "missing required parameter 'new_source'",
                ))
            }
        };

        let cell_type = input.get("cell_type").and_then(|v| v.as_str());
        if let Some(ct) = cell_type {
            if !VALID_CELL_TYPES.contains(&ct) {
                return Ok(ToolResult::error(format!(
                    "invalid cell_type '{ct}': must be one of {}",
                    VALID_CELL_TYPES.join(", ")
                )));
            }
        }

        let mode = input
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("replace");
        if !VALID_MODES.contains(&mode) {
            return Ok(ToolResult::error(format!(
                "invalid mode '{mode}': must be one of {}",
                VALID_MODES.join(", ")
            )));
        }

        // Validate notebook path extension
        if !notebook_path.ends_with(".ipynb") {
            return Ok(ToolResult::error("notebook_path must end with .ipynb"));
        }

        // Resolve path
        let path = if Path::new(notebook_path).is_absolute() {
            notebook_path.to_string()
        } else {
            Path::new(&ctx.cwd)
                .join(notebook_path)
                .to_string_lossy()
                .to_string()
        };

        // Read the notebook file
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Failed to read notebook {path}: {e}"
                )))
            }
        };

        // Parse as JSON
        let mut notebook: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Failed to parse notebook JSON: {e}"
                )))
            }
        };

        // Get cells array
        let cells = match notebook.get_mut("cells").and_then(|c| c.as_array_mut()) {
            Some(c) => c,
            None => {
                return Ok(ToolResult::error(
                    "Invalid notebook: missing or invalid 'cells' array",
                ))
            }
        };

        let num_cells = cells.len();

        match mode {
            "replace" => {
                if cell_index >= num_cells {
                    return Ok(ToolResult::error(format!(
                        "cell_index {cell_index} out of range (notebook has {num_cells} cells)"
                    )));
                }
                // Determine cell type: use provided type, or preserve existing
                let target_type = cell_type
                    .map(String::from)
                    .or_else(|| cells[cell_index]["cell_type"].as_str().map(String::from))
                    .unwrap_or_else(|| "code".to_string());

                let new_cell = make_cell(&target_type, new_source);
                // Preserve metadata from original cell if present
                if let Some(meta) = cells[cell_index].get("metadata").cloned() {
                    cells[cell_index] = new_cell;
                    cells[cell_index]["metadata"] = meta;
                } else {
                    cells[cell_index] = new_cell;
                }
            }
            "insert_before" => {
                if cell_index > num_cells {
                    return Ok(ToolResult::error(format!(
                        "cell_index {cell_index} out of range for insert \
                         (notebook has {num_cells} cells)"
                    )));
                }
                let target_type = cell_type.unwrap_or("code");
                let new_cell = make_cell(target_type, new_source);
                cells.insert(cell_index, new_cell);
            }
            "insert_after" => {
                if cell_index >= num_cells {
                    return Ok(ToolResult::error(format!(
                        "cell_index {cell_index} out of range (notebook has {num_cells} cells)"
                    )));
                }
                let target_type = cell_type.unwrap_or("code");
                let new_cell = make_cell(target_type, new_source);
                cells.insert(cell_index + 1, new_cell);
            }
            "delete" => {
                if cell_index >= num_cells {
                    return Ok(ToolResult::error(format!(
                        "cell_index {cell_index} out of range (notebook has {num_cells} cells)"
                    )));
                }
                cells.remove(cell_index);
            }
            _ => unreachable!(),
        }

        // Write back with pretty formatting
        let output_json = serde_json::to_string_pretty(&notebook)
            .map_err(|e| anyhow::anyhow!("Failed to serialize notebook: {e}"))?;

        if let Err(e) = tokio::fs::write(&path, &output_json).await {
            return Ok(ToolResult::error(format!(
                "Failed to write notebook {path}: {e}"
            )));
        }

        let final_cell_count = notebook["cells"]
            .as_array()
            .map(|c| c.len())
            .unwrap_or(0);

        Ok(ToolResult::success(format!(
            "Notebook {path} updated: {mode} at cell index {cell_index} \
             ({final_cell_count} cells total)"
        )))
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

    /// Create a minimal valid notebook JSON.
    fn make_test_notebook() -> serde_json::Value {
        json!({
            "nbformat": 4,
            "nbformat_minor": 5,
            "metadata": {
                "kernelspec": {
                    "display_name": "Python 3",
                    "language": "python",
                    "name": "python3"
                }
            },
            "cells": [
                {
                    "cell_type": "markdown",
                    "metadata": {},
                    "source": ["# Hello\n", "This is a test notebook."]
                },
                {
                    "cell_type": "code",
                    "execution_count": 1,
                    "metadata": {},
                    "outputs": [],
                    "source": ["print('hello')"]
                },
                {
                    "cell_type": "code",
                    "execution_count": null,
                    "metadata": {},
                    "outputs": [],
                    "source": ["x = 42"]
                }
            ]
        })
    }

    #[tokio::test]
    async fn test_notebook_replace_cell() {
        let dir = tempfile::tempdir().unwrap();
        let nb_path = dir.path().join("test.ipynb");
        let notebook = make_test_notebook();
        std::fs::write(&nb_path, serde_json::to_string_pretty(&notebook).unwrap()).unwrap();

        let tool = NotebookEditTool;
        let result = tool
            .call(
                json!({
                    "notebook_path": nb_path.to_string_lossy(),
                    "cell_index": 1,
                    "new_source": "print('updated')"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("replace"));
        assert!(result.output.contains("3 cells total"));

        // Verify the notebook was actually updated
        let content = std::fs::read_to_string(&nb_path).unwrap();
        let nb: serde_json::Value = serde_json::from_str(&content).unwrap();
        let source = nb["cells"][1]["source"][0].as_str().unwrap();
        assert_eq!(source, "print('updated')");
        // Should preserve existing cell type (code)
        assert_eq!(nb["cells"][1]["cell_type"], "code");
    }

    #[tokio::test]
    async fn test_notebook_insert_after() {
        let dir = tempfile::tempdir().unwrap();
        let nb_path = dir.path().join("test.ipynb");
        let notebook = make_test_notebook();
        std::fs::write(&nb_path, serde_json::to_string_pretty(&notebook).unwrap()).unwrap();

        let tool = NotebookEditTool;
        let result = tool
            .call(
                json!({
                    "notebook_path": nb_path.to_string_lossy(),
                    "cell_index": 0,
                    "new_source": "## New Section",
                    "cell_type": "markdown",
                    "mode": "insert_after"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("4 cells total"));

        let content = std::fs::read_to_string(&nb_path).unwrap();
        let nb: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(nb["cells"].as_array().unwrap().len(), 4);
        assert_eq!(nb["cells"][1]["cell_type"], "markdown");
    }

    #[tokio::test]
    async fn test_notebook_insert_before() {
        let dir = tempfile::tempdir().unwrap();
        let nb_path = dir.path().join("test.ipynb");
        let notebook = make_test_notebook();
        std::fs::write(&nb_path, serde_json::to_string_pretty(&notebook).unwrap()).unwrap();

        let tool = NotebookEditTool;
        let result = tool
            .call(
                json!({
                    "notebook_path": nb_path.to_string_lossy(),
                    "cell_index": 1,
                    "new_source": "import os",
                    "cell_type": "code",
                    "mode": "insert_before"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("4 cells total"));

        let content = std::fs::read_to_string(&nb_path).unwrap();
        let nb: serde_json::Value = serde_json::from_str(&content).unwrap();
        // Inserted cell is now at index 1, original cell 1 shifted to index 2
        assert_eq!(nb["cells"][1]["source"][0], "import os");
        assert_eq!(nb["cells"][2]["source"][0], "print('hello')");
    }

    #[tokio::test]
    async fn test_notebook_delete_cell() {
        let dir = tempfile::tempdir().unwrap();
        let nb_path = dir.path().join("test.ipynb");
        let notebook = make_test_notebook();
        std::fs::write(&nb_path, serde_json::to_string_pretty(&notebook).unwrap()).unwrap();

        let tool = NotebookEditTool;
        let result = tool
            .call(
                json!({
                    "notebook_path": nb_path.to_string_lossy(),
                    "cell_index": 2,
                    "new_source": "",
                    "mode": "delete"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        assert!(result.output.contains("2 cells total"));
    }

    #[tokio::test]
    async fn test_notebook_cell_index_out_of_range() {
        let dir = tempfile::tempdir().unwrap();
        let nb_path = dir.path().join("test.ipynb");
        let notebook = make_test_notebook();
        std::fs::write(&nb_path, serde_json::to_string_pretty(&notebook).unwrap()).unwrap();

        let tool = NotebookEditTool;
        let result = tool
            .call(
                json!({
                    "notebook_path": nb_path.to_string_lossy(),
                    "cell_index": 10,
                    "new_source": "oops"
                }),
                &ctx(dir.path()),
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output.contains("out of range"));
    }

    #[tokio::test]
    async fn test_notebook_invalid_extension() {
        let tool = NotebookEditTool;
        let ctx = ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
            progress_tx: None,
        };
        let result = tool
            .call(
                json!({
                    "notebook_path": "/tmp/test.py",
                    "cell_index": 0,
                    "new_source": "test"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output.contains(".ipynb"));
    }

    #[tokio::test]
    async fn test_notebook_nonexistent_file() {
        let tool = NotebookEditTool;
        let ctx = ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
            progress_tx: None,
        };
        let result = tool
            .call(
                json!({
                    "notebook_path": "/tmp/nonexistent_abc123.ipynb",
                    "cell_index": 0,
                    "new_source": "test"
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output.contains("Failed to read"));
    }

    #[test]
    fn test_string_to_source_multiline() {
        let source = string_to_source("line1\nline2\nline3");
        assert_eq!(source.len(), 3);
        assert_eq!(source[0], "line1\n");
        assert_eq!(source[1], "line2\n");
        assert_eq!(source[2], "line3");
    }

    #[test]
    fn test_string_to_source_single_line() {
        let source = string_to_source("print('hello')");
        assert_eq!(source.len(), 1);
        assert_eq!(source[0], "print('hello')");
    }

    #[test]
    fn test_make_cell_code() {
        let cell = make_cell("code", "x = 1");
        assert_eq!(cell["cell_type"], "code");
        assert!(cell["outputs"].is_array());
        assert!(cell.get("execution_count").is_some());
    }

    #[test]
    fn test_make_cell_markdown() {
        let cell = make_cell("markdown", "# Hello");
        assert_eq!(cell["cell_type"], "markdown");
        assert!(cell.get("outputs").is_none());
    }

    #[test]
    fn test_notebook_schema() {
        let tool = NotebookEditTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("notebook_path")));
        assert!(required.contains(&json!("cell_index")));
        assert!(required.contains(&json!("new_source")));
    }

    #[test]
    fn test_notebook_permission_is_workspace_write() {
        let tool = NotebookEditTool;
        assert_eq!(tool.permission_level(), PermissionLevel::WorkspaceWrite);
    }
}
