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
                },
                "pages": {
                    "type": "string",
                    "description": "Page range for PDF files (e.g., '1-5', '3', '10-20'). Max 20 pages per request."
                }
            },
            "required": ["file_path"]
        })
    }

    async fn call(&self, input: serde_json::Value, ctx: &ToolContext) -> Result<ToolResult> {
        let file_path = input["file_path"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'file_path'"))?;

        // Resolve relative paths against cwd
        let path = if Path::new(file_path).is_absolute() {
            file_path.to_string()
        } else {
            Path::new(&ctx.cwd)
                .join(file_path)
                .to_string_lossy()
                .to_string()
        };

        let ext = Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        // Handle PDF files
        if ext == "pdf" {
            return read_pdf(&path, input["pages"].as_str()).await;
        }

        // Handle image files — return base64-encoded content
        if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg") {
            return read_image(&path, &ext).await;
        }

        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().unwrap_or(2000) as usize;

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

    fn is_concurrency_safe(&self) -> bool {
        true
    }
}

/// Read a PDF file by shelling out to `pdftotext`.
async fn read_pdf(path: &str, pages: Option<&str>) -> Result<ToolResult> {
    use std::process::Stdio;
    use tokio::process::Command;

    // Parse page range
    let (first, last) = if let Some(page_str) = pages {
        parse_page_range(page_str)?
    } else {
        // Default: first 10 pages
        (1, 10)
    };

    // Enforce max 20 pages per request
    if last - first + 1 > 20 {
        return Ok(ToolResult::error(
            "Maximum 20 pages per request. Use the 'pages' parameter to specify a smaller range."
        ));
    }

    let args = vec![
        "-f".to_string(), first.to_string(),
        "-l".to_string(), last.to_string(),
        "-layout".to_string(),
        path.to_string(),
        "-".to_string(), // output to stdout
    ];

    let output = Command::new("pdftotext")
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await;

    match output {
        Ok(out) => {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                if text.trim().is_empty() {
                    Ok(ToolResult::success(format!(
                        "(PDF pages {first}-{last} contain no extractable text)"
                    )))
                } else {
                    Ok(ToolResult::success(format!(
                        "[PDF: {path}, pages {first}-{last}]\n\n{text}"
                    )))
                }
            } else {
                let stderr = String::from_utf8_lossy(&out.stderr);
                Ok(ToolResult::error(format!(
                    "pdftotext failed: {stderr}"
                )))
            }
        }
        Err(_) => {
            Ok(ToolResult::error(
                "Failed to run pdftotext. Install poppler-utils (apt install poppler-utils / brew install poppler) to read PDF files."
            ))
        }
    }
}

/// Parse a page range string like "1-5", "3", "10-20".
fn parse_page_range(s: &str) -> Result<(u32, u32)> {
    if let Some((start, end)) = s.split_once('-') {
        let first: u32 = start.trim().parse()
            .map_err(|_| anyhow::anyhow!("Invalid page number: '{start}'"))?;
        let last: u32 = end.trim().parse()
            .map_err(|_| anyhow::anyhow!("Invalid page number: '{end}'"))?;
        if first > last {
            anyhow::bail!("Invalid page range: start ({first}) > end ({last})");
        }
        Ok((first, last))
    } else {
        let page: u32 = s.trim().parse()
            .map_err(|_| anyhow::anyhow!("Invalid page number: '{s}'"))?;
        Ok((page, page))
    }
}

/// Read an image file and return base64-encoded content.
async fn read_image(path: &str, ext: &str) -> Result<ToolResult> {
    let data = match tokio::fs::read(path).await {
        Ok(d) => d,
        Err(e) => return Ok(ToolResult::error(format!("Failed to read image {path}: {e}"))),
    };

    // Size check — don't send huge images
    let size_mb = data.len() as f64 / (1024.0 * 1024.0);
    if size_mb > 10.0 {
        return Ok(ToolResult::error(format!(
            "Image too large ({size_mb:.1}MB). Maximum is 10MB."
        )));
    }

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&data);

    let media_type = match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    };

    Ok(ToolResult::success(format!(
        "[Image: {path} ({:.1}KB, {media_type})]\ndata:{media_type};base64,{encoded}",
        data.len() as f64 / 1024.0,
    )))
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
            progress_tx: None,
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

    #[test]
    fn test_parse_page_range_single() {
        let (f, l) = super::parse_page_range("5").unwrap();
        assert_eq!((f, l), (5, 5));
    }

    #[test]
    fn test_parse_page_range_range() {
        let (f, l) = super::parse_page_range("3-10").unwrap();
        assert_eq!((f, l), (3, 10));
    }

    #[test]
    fn test_parse_page_range_invalid() {
        assert!(super::parse_page_range("abc").is_err());
        assert!(super::parse_page_range("5-3").is_err());
    }

    #[tokio::test]
    async fn test_read_image_returns_base64() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.png");
        // Write a minimal 1x1 PNG
        let png_data: Vec<u8> = vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, // PNG signature
            0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, // IHDR chunk
            0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
            0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
            0xde,
        ];
        std::fs::write(&path, &png_data).unwrap();

        let tool = ReadTool;
        let result = tool
            .call(
                serde_json::json!({"file_path": path.to_string_lossy()}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("image/png"));
        assert!(result.output.contains("base64,"));
    }

    #[tokio::test]
    async fn test_read_pdf_without_pdftotext() {
        // This test verifies the error message when pdftotext is not installed
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.pdf");
        std::fs::write(&path, "%PDF-1.4 fake pdf content").unwrap();

        let tool = ReadTool;
        let result = tool
            .call(
                serde_json::json!({"file_path": path.to_string_lossy()}),
                &ctx(dir.path()),
            )
            .await
            .unwrap();
        // Either succeeds (pdftotext installed) or gives a helpful error
        if result.is_error {
            assert!(
                result.output.contains("pdftotext")
                    || result.output.contains("poppler")
                    || result.output.contains("Failed")
            );
        }
    }
}
