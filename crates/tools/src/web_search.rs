//! WebSearch tool — search the web for information.
//!
//! Uses the Brave Search API. Set the `BRAVE_SEARCH_API_KEY` environment
//! variable to enable web search.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

use crate::{Tool, ToolContext};

pub struct WebSearchTool;

/// A single search result returned by the Brave Search API.
#[derive(Debug, Deserialize)]
struct SearchResult {
    title: String,
    url: String,
    description: String,
}

/// Top-level Brave Search API response (only the fields we need).
#[derive(Debug, Deserialize)]
struct BraveSearchResponse {
    web: Option<BraveWebResults>,
}

#[derive(Debug, Deserialize)]
struct BraveWebResults {
    results: Vec<SearchResult>,
}

/// Call the Brave Search API and return parsed results.
async fn search_brave(query: &str, max_results: usize) -> Result<Vec<SearchResult>> {
    let api_key = std::env::var("BRAVE_SEARCH_API_KEY")
        .map_err(|_| anyhow::anyhow!("BRAVE_SEARCH_API_KEY not set"))?;

    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.search.brave.com/res/v1/web/search")
        .header("X-Subscription-Token", &api_key)
        .header("Accept", "application/json")
        .query(&[("q", query), ("count", &max_results.to_string())])
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Brave Search API returned {status}: {body}");
    }

    let parsed: BraveSearchResponse = resp.json().await?;
    Ok(parsed.web.map(|w| w.results).unwrap_or_default())
}

/// Format search results into a readable numbered list.
fn format_results(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No results found.".to_string();
    }
    let mut out = String::new();
    for (i, r) in results.iter().enumerate() {
        let _ = writeln!(out, "{}. {}", i + 1, r.title);
        let _ = writeln!(out, "   {}", r.url);
        let _ = writeln!(out, "   {}", r.description);
        if i + 1 < results.len() {
            let _ = writeln!(out);
        }
    }
    out
}

#[async_trait::async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }

    fn description(&self) -> &str {
        "Search the web for information. Requires a configured search API provider. Returns search results with titles, URLs, and snippets."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 5)"
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let query = match input["query"].as_str() {
            Some(q) if !q.trim().is_empty() => q,
            Some(_) => return Ok(ToolResult::error("'query' must not be empty")),
            None => return Ok(ToolResult::error("missing required parameter 'query'")),
        };

        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(5);

        // Validate max_results range
        if max_results == 0 {
            return Ok(ToolResult::error("'max_results' must be at least 1"));
        }
        if max_results > 50 {
            return Ok(ToolResult::error("'max_results' must be at most 50"));
        }

        match search_brave(query, max_results as usize).await {
            Ok(results) => Ok(ToolResult::success(format_results(&results))),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("BRAVE_SEARCH_API_KEY not set") {
                    Ok(ToolResult::error(
                        "Web search requires the BRAVE_SEARCH_API_KEY environment variable. \
                         Get a free API key at https://brave.com/search/api/ and set it:\n\n  \
                         export BRAVE_SEARCH_API_KEY=\"your-key-here\"",
                    ))
                } else {
                    Ok(ToolResult::error(format!("Web search failed: {msg}")))
                }
            }
        }
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
    async fn test_web_search_no_api_key() {
        // This test verifies the "no API key" error path.
        // Skip if the env var is already set — we cannot remove it
        // because remove_var is unsafe in Rust 2024 and unsafe_code is forbidden.
        if std::env::var("BRAVE_SEARCH_API_KEY").is_ok() {
            eprintln!("BRAVE_SEARCH_API_KEY is set, skipping no-api-key test");
            return;
        }

        let tool = WebSearchTool;
        let result = tool
            .call(json!({"query": "rust async programming"}), &ctx())
            .await
            .unwrap();

        assert!(result.is_error);
        assert!(result.output.contains("BRAVE_SEARCH_API_KEY"));
        assert!(result.output.contains("https://brave.com/search/api/"));
    }

    #[tokio::test]
    async fn test_web_search_no_api_key_with_max_results() {
        // This test verifies the "no API key" error path with max_results.
        // Skip if the env var is already set — we cannot remove it
        // because remove_var is unsafe in Rust 2024 and unsafe_code is forbidden.
        if std::env::var("BRAVE_SEARCH_API_KEY").is_ok() {
            eprintln!("BRAVE_SEARCH_API_KEY is set, skipping no-api-key test");
            return;
        }

        let tool = WebSearchTool;
        let result = tool
            .call(
                json!({"query": "cisco networking", "max_results": 10}),
                &ctx(),
            )
            .await
            .unwrap();

        // Without an API key the tool should return the helpful error
        assert!(result.is_error);
        assert!(result.output.contains("BRAVE_SEARCH_API_KEY"));
    }

    #[tokio::test]
    async fn test_web_search_missing_query() {
        let tool = WebSearchTool;
        let result = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("query"));
    }

    #[tokio::test]
    async fn test_web_search_empty_query() {
        let tool = WebSearchTool;
        let result = tool.call(json!({"query": "  "}), &ctx()).await.unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("empty"));
    }

    #[tokio::test]
    async fn test_web_search_max_results_zero() {
        let tool = WebSearchTool;
        let result = tool
            .call(json!({"query": "test", "max_results": 0}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("at least 1"));
    }

    #[tokio::test]
    async fn test_web_search_max_results_too_large() {
        let tool = WebSearchTool;
        let result = tool
            .call(json!({"query": "test", "max_results": 100}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("at most 50"));
    }

    #[test]
    fn test_web_search_schema() {
        let tool = WebSearchTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("query")));
    }

    #[test]
    fn test_web_search_permission_is_execute() {
        let tool = WebSearchTool;
        assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    }

    #[test]
    fn test_format_results_empty() {
        let results: Vec<SearchResult> = vec![];
        assert_eq!(format_results(&results), "No results found.");
    }

    #[test]
    fn test_format_results_multiple() {
        let results = vec![
            SearchResult {
                title: "First Result".into(),
                url: "https://example.com/1".into(),
                description: "Description one".into(),
            },
            SearchResult {
                title: "Second Result".into(),
                url: "https://example.com/2".into(),
                description: "Description two".into(),
            },
        ];
        let formatted = format_results(&results);
        assert!(formatted.contains("1. First Result"));
        assert!(formatted.contains("   https://example.com/1"));
        assert!(formatted.contains("   Description one"));
        assert!(formatted.contains("2. Second Result"));
        assert!(formatted.contains("   https://example.com/2"));
        assert!(formatted.contains("   Description two"));
    }
}
