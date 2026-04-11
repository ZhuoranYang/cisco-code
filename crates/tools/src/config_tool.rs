//! Config tool — get and set cisco-code settings.
//!
//! Matches Claude Code's ConfigTool: allows reading and writing
//! configuration settings like model, theme, permissions, etc.

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

pub struct ConfigTool;

/// Settings that can be read/written.
const KNOWN_SETTINGS: &[&str] = &[
    "model",
    "theme",
    "permissions.default_mode",
    "permissions.bypass_mode",
    "sandbox.profile",
    "max_tokens",
    "temperature",
    "verbose",
    "telemetry.enabled",
    "hooks.enabled",
    "memory.enabled",
];

#[async_trait::async_trait]
impl Tool for ConfigTool {
    fn name(&self) -> &str {
        "Config"
    }

    fn description(&self) -> &str {
        "Read or write cisco-code configuration settings. Omit 'value' to read the current value of a setting."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "setting": {
                    "type": "string",
                    "description": "Setting key (e.g., 'model', 'theme', 'permissions.default_mode')"
                },
                "value": {
                    "description": "New value to set. Omit to read current value.",
                }
            },
            "required": ["setting"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let setting = match input["setting"].as_str() {
            Some(s) if !s.trim().is_empty() => s.trim(),
            Some(_) => return Ok(ToolResult::error("'setting' must not be empty")),
            None => return Ok(ToolResult::error("missing required parameter 'setting'")),
        };

        let value = input.get("value").filter(|v| !v.is_null());
        let is_read = value.is_none();

        // Build config request for runtime to handle
        let request = if is_read {
            json!({
                "type": "config_read",
                "setting": setting,
                "known": KNOWN_SETTINGS.contains(&setting),
            })
        } else {
            json!({
                "type": "config_write",
                "setting": setting,
                "value": value,
                "known": KNOWN_SETTINGS.contains(&setting),
            })
        };

        Ok(ToolResult::success(
            serde_json::to_string_pretty(&request)
                .unwrap_or_else(|_| request.to_string()),
        ))
    }

    fn permission_level(&self) -> PermissionLevel {
        // Reads are safe; writes require permission (handled by runtime)
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
    async fn test_config_read() {
        let tool = ConfigTool;
        let result = tool
            .call(json!({"setting": "model"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["type"], "config_read");
        assert_eq!(parsed["setting"], "model");
        assert_eq!(parsed["known"], true);
    }

    #[tokio::test]
    async fn test_config_write() {
        let tool = ConfigTool;
        let result = tool
            .call(
                json!({"setting": "model", "value": "claude-opus-4-6"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert!(!result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["type"], "config_write");
        assert_eq!(parsed["value"], "claude-opus-4-6");
    }

    #[tokio::test]
    async fn test_config_unknown_setting() {
        let tool = ConfigTool;
        let result = tool
            .call(json!({"setting": "unknown.key"}), &ctx())
            .await
            .unwrap();
        assert!(!result.is_error);
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["known"], false);
    }

    #[tokio::test]
    async fn test_config_missing_setting() {
        let tool = ConfigTool;
        let result = tool.call(json!({}), &ctx()).await.unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("setting"));
    }

    #[tokio::test]
    async fn test_config_empty_setting() {
        let tool = ConfigTool;
        let result = tool
            .call(json!({"setting": "  "}), &ctx())
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_config_null_value_is_read() {
        let tool = ConfigTool;
        let result = tool
            .call(json!({"setting": "theme", "value": null}), &ctx())
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["type"], "config_read");
    }

    #[test]
    fn test_config_schema() {
        let tool = ConfigTool;
        let schema = tool.input_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("setting")));
        assert!(!required.contains(&json!("value")));
    }

    #[test]
    fn test_config_known_settings() {
        assert!(KNOWN_SETTINGS.contains(&"model"));
        assert!(KNOWN_SETTINGS.contains(&"theme"));
        assert!(!KNOWN_SETTINGS.contains(&"foobar"));
    }

    #[test]
    fn test_config_permission() {
        assert_eq!(ConfigTool.permission_level(), PermissionLevel::Execute);
    }
}
