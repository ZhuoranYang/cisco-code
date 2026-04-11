//! ToolSearch tool — on-demand tool schema loading.
//!
//! Matches Claude Code's ToolSearch / deferred tool mechanism.
//! Fetches full schemas for deferred tools that were not loaded initially.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

pub struct ToolSearchTool;

#[async_trait::async_trait]
impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        "ToolSearch"
    }

    fn description(&self) -> &str {
        "Fetch full schema definitions for deferred tools. Use 'select:Tool1,Tool2' for exact matches or keywords for search."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Query: 'select:Name1,Name2' for exact match, or keywords for search"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum results to return (default 5)"
                }
            },
            "required": ["query"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let query = input["query"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'query' parameter"))?;

        let max_results = input["max_results"].as_u64().unwrap_or(5) as usize;

        if query.trim().is_empty() {
            return Ok(ToolResult::error("Query cannot be empty".to_string()));
        }

        // Parse the query
        let search_type = if let Some(names) = query.strip_prefix("select:") {
            let tool_names: Vec<&str> = names.split(',').map(|s| s.trim()).collect();
            SearchType::Exact(tool_names)
        } else {
            SearchType::Keyword(query.to_string())
        };

        // Build the search request (runtime handles actual lookup)
        let request = json!({
            "action": "tool_search",
            "search_type": match &search_type {
                SearchType::Exact(names) => json!({"exact": names}),
                SearchType::Keyword(kw) => json!({"keyword": kw}),
            },
            "max_results": max_results,
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

enum SearchType<'a> {
    Exact(Vec<&'a str>),
    Keyword(String),
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
    async fn test_tool_search_exact() {
        let tool = ToolSearchTool;
        let result = tool
            .call(json!({"query": "select:Read,Edit,Grep"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("tool_search"));
        assert!(result.output.contains("exact"));
    }

    #[tokio::test]
    async fn test_tool_search_keyword() {
        let tool = ToolSearchTool;
        let result = tool
            .call(json!({"query": "notebook jupyter"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output.contains("keyword"));
    }

    #[tokio::test]
    async fn test_tool_search_empty_query() {
        let tool = ToolSearchTool;
        let result = tool
            .call(json!({"query": ""}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_tool_search_missing_query() {
        let tool = ToolSearchTool;
        let result = tool.call(json!({}), &ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tool_search_with_max_results() {
        let tool = ToolSearchTool;
        let result = tool
            .call(json!({"query": "test", "max_results": 10}), &ctx())
            .await
            .unwrap();
        assert!(result.output.contains("10"));
    }

    #[test]
    fn test_tool_search_schema() {
        let tool = ToolSearchTool;
        let schema = tool.input_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("query")));
    }
}
