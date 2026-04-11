//! Agent tool — spawn subagent tasks.
//!
//! This is a "request" tool: it packages a subagent request as JSON
//! for the runtime layer to dispatch. The tool itself does not execute
//! the subagent — it validates parameters and returns a structured request.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

pub struct AgentTool;

const VALID_SUBAGENT_TYPES: &[&str] = &["general-purpose", "Explore", "Plan"];
const VALID_ISOLATION_MODES: &[&str] = &["worktree"];

#[async_trait::async_trait]
impl Tool for AgentTool {
    fn name(&self) -> &str {
        "Agent"
    }

    fn description(&self) -> &str {
        "Spawn a subagent to handle a task. The subagent runs in its own context and returns results when complete. Use for parallelizable tasks, exploration, or planning."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The task for the subagent to perform"
                },
                "description": {
                    "type": "string",
                    "description": "Short 3-5 word description of the task"
                },
                "subagent_type": {
                    "type": "string",
                    "description": "Type of subagent: 'general-purpose' (default), 'Explore', or 'Plan'",
                    "enum": ["general-purpose", "Explore", "Plan"]
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "Whether to run the agent in the background (default: false)"
                },
                "model": {
                    "type": "string",
                    "description": "Model override for the subagent"
                },
                "isolation": {
                    "type": "string",
                    "description": "Isolation mode: 'worktree' for git worktree isolation",
                    "enum": ["worktree"]
                }
            },
            "required": ["prompt", "description"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let prompt = match input["prompt"].as_str() {
            Some(p) if !p.trim().is_empty() => p,
            Some(_) => return Ok(ToolResult::error("'prompt' must not be empty")),
            None => return Ok(ToolResult::error("missing required parameter 'prompt'")),
        };

        let description = match input["description"].as_str() {
            Some(d) if !d.trim().is_empty() => d,
            Some(_) => return Ok(ToolResult::error("'description' must not be empty")),
            None => {
                return Ok(ToolResult::error(
                    "missing required parameter 'description'",
                ))
            }
        };

        let subagent_type = match input.get("subagent_type").and_then(|v| v.as_str()) {
            Some(t) if VALID_SUBAGENT_TYPES.contains(&t) => t.to_string(),
            Some(t) => {
                return Ok(ToolResult::error(format!(
                    "invalid subagent_type '{t}': must be one of {}",
                    VALID_SUBAGENT_TYPES.join(", ")
                )))
            }
            None => "general-purpose".to_string(),
        };

        let run_in_background = input
            .get("run_in_background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let model = input.get("model").and_then(|v| v.as_str()).map(String::from);

        let isolation = match input.get("isolation").and_then(|v| v.as_str()) {
            Some(i) if VALID_ISOLATION_MODES.contains(&i) => Some(i.to_string()),
            Some(i) => {
                return Ok(ToolResult::error(format!(
                    "invalid isolation mode '{i}': must be one of {}",
                    VALID_ISOLATION_MODES.join(", ")
                )))
            }
            None => None,
        };

        // Build the request payload for the runtime to dispatch
        let mut request = json!({
            "type": "subagent_request",
            "prompt": prompt,
            "description": description,
            "subagent_type": subagent_type,
            "run_in_background": run_in_background,
        });

        if let Some(m) = &model {
            request["model"] = json!(m);
        }
        if let Some(iso) = &isolation {
            request["isolation"] = json!(iso);
        }

        Ok(ToolResult::success(
            serde_json::to_string_pretty(&request)
                .unwrap_or_else(|_| request.to_string()),
        ))
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
    async fn test_agent_basic_request() {
        let tool = AgentTool;
        let result = tool
            .call(
                json!({
                    "prompt": "Find all TODO comments in the codebase",
                    "description": "Find TODO comments"
                }),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["type"], "subagent_request");
        assert_eq!(parsed["prompt"], "Find all TODO comments in the codebase");
        assert_eq!(parsed["description"], "Find TODO comments");
        assert_eq!(parsed["subagent_type"], "general-purpose");
        assert_eq!(parsed["run_in_background"], false);
    }

    #[tokio::test]
    async fn test_agent_with_all_options() {
        let tool = AgentTool;
        let result = tool
            .call(
                json!({
                    "prompt": "Explore the project structure",
                    "description": "Explore project",
                    "subagent_type": "Explore",
                    "run_in_background": true,
                    "model": "claude-opus-4-0520",
                    "isolation": "worktree"
                }),
                &ctx(),
            )
            .await
            .unwrap();

        assert!(!result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["subagent_type"], "Explore");
        assert_eq!(parsed["run_in_background"], true);
        assert_eq!(parsed["model"], "claude-opus-4-0520");
        assert_eq!(parsed["isolation"], "worktree");
    }

    #[tokio::test]
    async fn test_agent_missing_prompt() {
        let tool = AgentTool;
        let result = tool
            .call(json!({"description": "test task"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("prompt"));
    }

    #[tokio::test]
    async fn test_agent_missing_description() {
        let tool = AgentTool;
        let result = tool
            .call(json!({"prompt": "do something"}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("description"));
    }

    #[tokio::test]
    async fn test_agent_invalid_subagent_type() {
        let tool = AgentTool;
        let result = tool
            .call(
                json!({
                    "prompt": "do something",
                    "description": "test",
                    "subagent_type": "invalid"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("invalid subagent_type"));
    }

    #[tokio::test]
    async fn test_agent_invalid_isolation_mode() {
        let tool = AgentTool;
        let result = tool
            .call(
                json!({
                    "prompt": "do something",
                    "description": "test",
                    "isolation": "sandbox"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("invalid isolation mode"));
    }

    #[test]
    fn test_agent_schema() {
        let tool = AgentTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("prompt")));
        assert!(required.contains(&json!("description")));
    }

    #[test]
    fn test_agent_permission_is_execute() {
        let tool = AgentTool;
        assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    }

    #[tokio::test]
    async fn test_agent_plan_subagent_type() {
        let tool = AgentTool;
        let result = tool
            .call(
                json!({
                    "prompt": "Plan the refactor",
                    "description": "Plan refactor",
                    "subagent_type": "Plan"
                }),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["subagent_type"], "Plan");
    }

    #[tokio::test]
    async fn test_agent_empty_prompt_rejected() {
        let tool = AgentTool;
        let result = tool
            .call(
                json!({"prompt": "  ", "description": "test"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("empty"));
    }

    #[tokio::test]
    async fn test_agent_model_not_included_when_absent() {
        let tool = AgentTool;
        let result = tool
            .call(
                json!({"prompt": "task", "description": "task"}),
                &ctx(),
            )
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert!(parsed.get("model").is_none());
    }
}
