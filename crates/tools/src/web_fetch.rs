//! WebFetch tool — fetch URLs and extract text content.
//!
//! Fetches a URL via HTTP, strips HTML tags for readability,
//! and optionally applies a processing prompt as context.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use regex::Regex;
use serde_json::json;

use crate::{Tool, ToolContext};

pub struct WebFetchTool;

/// Strip HTML to extract readable text content.
fn strip_html(html: &str) -> String {
    // Remove <script> blocks
    let re_script = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let text = re_script.replace_all(html, "");

    // Remove <style> blocks
    let re_style = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let text = re_style.replace_all(&text, "");

    // Remove all remaining HTML tags
    let re_tags = Regex::new(r"<[^>]+>").unwrap();
    let text = re_tags.replace_all(&text, "");

    // Decode common HTML entities
    let text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    // Collapse multiple newlines
    let re_whitespace = Regex::new(r"\n{3,}").unwrap();
    let text = re_whitespace.replace_all(&text, "\n\n");

    text.trim().to_string()
}

/// Check if content looks like HTML based on content-type or content inspection.
fn is_html_content(content_type: Option<&str>, body: &str) -> bool {
    if let Some(ct) = content_type {
        if ct.contains("text/html") {
            return true;
        }
    }
    // Heuristic: starts with common HTML markers
    let trimmed = body.trim_start();
    trimmed.starts_with("<!DOCTYPE")
        || trimmed.starts_with("<!doctype")
        || trimmed.starts_with("<html")
}

impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "WebFetch"
    }

    fn description(&self) -> &str {
        "Fetches a URL and returns its content. HTML pages are automatically stripped to text. Use the prompt parameter to provide context about what to look for in the content."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "prompt": {
                    "type": "string",
                    "description": "Optional processing prompt — context about what to look for"
                },
                "max_size_kb": {
                    "type": "integer",
                    "description": "Maximum response size in KB (default: 1024)"
                }
            },
            "required": ["url"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let url = match input["url"].as_str() {
            Some(u) if !u.trim().is_empty() => u,
            Some(_) => return Ok(ToolResult::error("'url' must not be empty")),
            None => return Ok(ToolResult::error("missing required parameter 'url'")),
        };

        // Validate URL format
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Ok(ToolResult::error(
                "URL must start with http:// or https://",
            ));
        }

        let max_size_kb = input
            .get("max_size_kb")
            .and_then(|v| v.as_u64())
            .unwrap_or(1024);
        let max_bytes = max_size_kb * 1024;

        let prompt = input.get("prompt").and_then(|v| v.as_str());

        // Fetch the URL
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {e}"))?;

        let response = match client.get(url).send().await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::error(format!("Failed to fetch {url}: {e}"))),
        };

        let status = response.status();
        if !status.is_success() {
            return Ok(ToolResult::error(format!("HTTP {status} fetching {url}")));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        let body = match response.text().await {
            Ok(b) => b,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Failed to read response body: {e}"
                )))
            }
        };

        // Strip HTML if applicable
        let content = if is_html_content(content_type.as_deref(), &body) {
            strip_html(&body)
        } else {
            body
        };

        // Truncate to max size
        let content = if content.len() > max_bytes as usize {
            let truncated = &content[..max_bytes as usize];
            // Try to break at a word boundary
            let break_at = truncated.rfind(' ').unwrap_or(max_bytes as usize);
            format!(
                "{}\n\n... (truncated, {} bytes total)",
                &truncated[..break_at],
                content.len()
            )
        } else {
            content
        };

        // Build output
        let mut output = String::new();
        if let Some(p) = prompt {
            output.push_str(&format!("Prompt: {p}\n\n"));
        }
        output.push_str(&format!("URL: {url}\n"));
        if let Some(ct) = &content_type {
            output.push_str(&format!("Content-Type: {ct}\n"));
        }
        output.push_str(&format!("---\n{content}"));

        Ok(ToolResult::success(output))
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

    #[test]
    fn test_strip_html_basic() {
        let html = "<html><body><h1>Title</h1><p>Hello world</p></body></html>";
        let result = strip_html(html);
        assert!(result.contains("Title"));
        assert!(result.contains("Hello world"));
        assert!(!result.contains("<h1>"));
        assert!(!result.contains("<p>"));
    }

    #[test]
    fn test_strip_html_removes_script_and_style() {
        let html = r#"<html>
            <head><style>body { color: red; }</style></head>
            <body>
                <script>alert('xss');</script>
                <p>Visible content</p>
            </body>
        </html>"#;
        let result = strip_html(html);
        assert!(result.contains("Visible content"));
        assert!(!result.contains("alert"));
        assert!(!result.contains("color: red"));
    }

    #[test]
    fn test_strip_html_decodes_entities() {
        let html = "<p>A &amp; B &lt; C &gt; D &quot;E&quot;</p>";
        let result = strip_html(html);
        assert!(result.contains("A & B < C > D \"E\""));
    }

    #[test]
    fn test_is_html_content_detection() {
        assert!(is_html_content(Some("text/html; charset=utf-8"), "anything"));
        assert!(is_html_content(None, "<!DOCTYPE html><html>"));
        assert!(is_html_content(None, "<html><body>"));
        assert!(!is_html_content(
            Some("application/json"),
            r#"{"key": "value"}"#
        ));
        assert!(!is_html_content(None, "Just plain text"));
    }

    #[tokio::test]
    async fn test_web_fetch_missing_url() {
        let tool = WebFetchTool;
        let result = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("url"));
    }

    #[tokio::test]
    async fn test_web_fetch_invalid_url_scheme() {
        let tool = WebFetchTool;
        let result = tool
            .call(json!({"url": "ftp://example.com"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("http://"));
    }

    #[tokio::test]
    async fn test_web_fetch_empty_url() {
        let tool = WebFetchTool;
        let result = tool.call(json!({"url": ""}), &ctx()).await.unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("empty"));
    }

    #[test]
    fn test_web_fetch_schema() {
        let tool = WebFetchTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("url")));
        assert!(schema["properties"].get("prompt").is_some());
        assert!(schema["properties"].get("max_size_kb").is_some());
    }

    #[test]
    fn test_web_fetch_permission_is_execute() {
        let tool = WebFetchTool;
        assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    }
}
