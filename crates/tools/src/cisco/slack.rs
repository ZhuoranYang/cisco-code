//! Slack tool — interact with Slack workspaces via Web API.
//!
//! Supports:
//! - send_message: Post a message to a channel or DM
//! - list_channels: List channels the bot/user is in
//! - list_messages: List recent messages in a channel
//! - add_reaction: Add an emoji reaction to a message
//! - search_messages: Search messages across the workspace
//!
//! Auth: SLACK_TOKEN environment variable (Bot or User OAuth token).
//! API docs: https://api.slack.com/methods

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

const SLACK_BASE_URL: &str = "https://slack.com/api";

pub struct SlackTool;

impl SlackTool {
    fn get_token() -> Result<String> {
        std::env::var("SLACK_TOKEN").map_err(|_| {
            anyhow::anyhow!(
                "SLACK_TOKEN not set. Set it to your Slack Bot OAuth Token (xoxb-...) or User Token (xoxp-...)."
            )
        })
    }

    fn http_client() -> reqwest::Client {
        reqwest::Client::new()
    }
}

impl Tool for SlackTool {
    fn name(&self) -> &str {
        "Slack"
    }

    fn description(&self) -> &str {
        "Interact with Slack workspaces. Actions: send_message, list_channels, list_messages, add_reaction, search_messages. Requires SLACK_TOKEN env var."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["send_message", "list_channels", "list_messages", "add_reaction", "search_messages"],
                    "description": "The Slack action to perform"
                },
                "channel": {
                    "type": "string",
                    "description": "Channel ID (for send_message, list_messages, add_reaction)"
                },
                "text": {
                    "type": "string",
                    "description": "Message text (for send_message, search_messages)"
                },
                "thread_ts": {
                    "type": "string",
                    "description": "Thread timestamp to reply in (for send_message, optional)"
                },
                "timestamp": {
                    "type": "string",
                    "description": "Message timestamp (for add_reaction)"
                },
                "emoji": {
                    "type": "string",
                    "description": "Emoji name without colons (for add_reaction, e.g. 'thumbsup')"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (for search_messages)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Max results to return (default 10)"
                }
            },
            "required": ["action"]
        })
    }

    async fn call(&self, input: serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let token = match Self::get_token() {
            Ok(t) => t,
            Err(e) => return Ok(ToolResult::error(e.to_string())),
        };

        let action = input["action"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("missing 'action'"))?;

        let client = Self::http_client();
        let max_results = input["max_results"].as_u64().unwrap_or(10);

        match action {
            "send_message" => {
                let channel = match input["channel"].as_str() {
                    Some(c) => c,
                    None => {
                        return Ok(ToolResult::error("send_message requires channel"));
                    }
                };
                let text = match input["text"].as_str() {
                    Some(t) => t,
                    None => return Ok(ToolResult::error("send_message requires text")),
                };

                let mut body = json!({
                    "channel": channel,
                    "text": text,
                });
                if let Some(thread_ts) = input["thread_ts"].as_str() {
                    body["thread_ts"] = json!(thread_ts);
                }

                let resp = client
                    .post(format!("{SLACK_BASE_URL}/chat.postMessage"))
                    .bearer_auth(&token)
                    .json(&body)
                    .send()
                    .await;

                handle_slack_response(resp, "Message sent").await
            }

            "list_channels" => {
                let resp = client
                    .get(format!("{SLACK_BASE_URL}/conversations.list"))
                    .bearer_auth(&token)
                    .query(&[
                        ("limit", max_results.to_string()),
                        ("types", "public_channel,private_channel".into()),
                    ])
                    .send()
                    .await;

                match resp {
                    Ok(r) if r.status().is_success() => {
                        let body: serde_json::Value = r.json().await.unwrap_or_default();
                        if !body["ok"].as_bool().unwrap_or(false) {
                            let err = body["error"].as_str().unwrap_or("unknown error");
                            return Ok(ToolResult::error(format!("Slack API error: {err}")));
                        }

                        let channels = body["channels"].as_array();
                        match channels {
                            Some(items) => {
                                let summary: Vec<String> = items
                                    .iter()
                                    .map(|ch| {
                                        let name = ch["name"].as_str().unwrap_or("?");
                                        let id = ch["id"].as_str().unwrap_or("?");
                                        let members = ch["num_members"].as_u64().unwrap_or(0);
                                        let purpose = ch["purpose"]["value"]
                                            .as_str()
                                            .unwrap_or("")
                                            .chars()
                                            .take(60)
                                            .collect::<String>();
                                        format!("- #{name} ({id}) [{members} members] {purpose}")
                                    })
                                    .collect();
                                Ok(ToolResult::success(format!(
                                    "{} channel(s):\n{}",
                                    summary.len(),
                                    summary.join("\n")
                                )))
                            }
                            None => Ok(ToolResult::success("No channels found.")),
                        }
                    }
                    Ok(r) => {
                        let status = r.status();
                        let text = r.text().await.unwrap_or_default();
                        Ok(ToolResult::error(format!(
                            "Slack API error {status}: {text}"
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Request failed: {e}"))),
                }
            }

            "list_messages" => {
                let channel = match input["channel"].as_str() {
                    Some(c) => c,
                    None => {
                        return Ok(ToolResult::error("list_messages requires channel"));
                    }
                };

                let resp = client
                    .get(format!("{SLACK_BASE_URL}/conversations.history"))
                    .bearer_auth(&token)
                    .query(&[
                        ("channel", channel.to_string()),
                        ("limit", max_results.to_string()),
                    ])
                    .send()
                    .await;

                match resp {
                    Ok(r) if r.status().is_success() => {
                        let body: serde_json::Value = r.json().await.unwrap_or_default();
                        if !body["ok"].as_bool().unwrap_or(false) {
                            let err = body["error"].as_str().unwrap_or("unknown error");
                            return Ok(ToolResult::error(format!("Slack API error: {err}")));
                        }

                        let messages = body["messages"].as_array();
                        match messages {
                            Some(items) => {
                                let summary: Vec<String> = items
                                    .iter()
                                    .map(|msg| {
                                        let user = msg["user"].as_str().unwrap_or("bot");
                                        let text = msg["text"]
                                            .as_str()
                                            .unwrap_or("")
                                            .chars()
                                            .take(200)
                                            .collect::<String>();
                                        let ts = msg["ts"].as_str().unwrap_or("?");
                                        format!("[{ts}] {user}: {text}")
                                    })
                                    .collect();
                                Ok(ToolResult::success(format!(
                                    "{} message(s):\n{}",
                                    summary.len(),
                                    summary.join("\n")
                                )))
                            }
                            None => Ok(ToolResult::success("No messages found.")),
                        }
                    }
                    Ok(r) => {
                        let status = r.status();
                        let text = r.text().await.unwrap_or_default();
                        Ok(ToolResult::error(format!(
                            "Slack API error {status}: {text}"
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Request failed: {e}"))),
                }
            }

            "add_reaction" => {
                let channel = match input["channel"].as_str() {
                    Some(c) => c,
                    None => return Ok(ToolResult::error("add_reaction requires channel")),
                };
                let timestamp = match input["timestamp"].as_str() {
                    Some(t) => t,
                    None => return Ok(ToolResult::error("add_reaction requires timestamp")),
                };
                let emoji = match input["emoji"].as_str() {
                    Some(e) => e,
                    None => return Ok(ToolResult::error("add_reaction requires emoji")),
                };

                let resp = client
                    .post(format!("{SLACK_BASE_URL}/reactions.add"))
                    .bearer_auth(&token)
                    .json(&json!({
                        "channel": channel,
                        "timestamp": timestamp,
                        "name": emoji,
                    }))
                    .send()
                    .await;

                handle_slack_response(resp, &format!("Reaction :{emoji}: added")).await
            }

            "search_messages" => {
                let query = match input["query"].as_str() {
                    Some(q) => q,
                    None => return Ok(ToolResult::error("search_messages requires query")),
                };

                let resp = client
                    .get(format!("{SLACK_BASE_URL}/search.messages"))
                    .bearer_auth(&token)
                    .query(&[
                        ("query", query.to_string()),
                        ("count", max_results.to_string()),
                    ])
                    .send()
                    .await;

                match resp {
                    Ok(r) if r.status().is_success() => {
                        let body: serde_json::Value = r.json().await.unwrap_or_default();
                        if !body["ok"].as_bool().unwrap_or(false) {
                            let err = body["error"].as_str().unwrap_or("unknown error");
                            return Ok(ToolResult::error(format!("Slack API error: {err}")));
                        }

                        let matches = body["messages"]["matches"].as_array();
                        match matches {
                            Some(items) => {
                                let summary: Vec<String> = items
                                    .iter()
                                    .map(|msg| {
                                        let channel_name = msg["channel"]["name"]
                                            .as_str()
                                            .unwrap_or("?");
                                        let user = msg["username"].as_str().unwrap_or("?");
                                        let text = msg["text"]
                                            .as_str()
                                            .unwrap_or("")
                                            .chars()
                                            .take(150)
                                            .collect::<String>();
                                        format!("- #{channel_name} @{user}: {text}")
                                    })
                                    .collect();
                                Ok(ToolResult::success(format!(
                                    "{} result(s) for '{query}':\n{}",
                                    summary.len(),
                                    summary.join("\n")
                                )))
                            }
                            None => Ok(ToolResult::success(format!(
                                "No results for '{query}'."
                            ))),
                        }
                    }
                    Ok(r) => {
                        let status = r.status();
                        let text = r.text().await.unwrap_or_default();
                        Ok(ToolResult::error(format!(
                            "Slack API error {status}: {text}"
                        )))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Request failed: {e}"))),
                }
            }

            _ => Ok(ToolResult::error(format!(
                "Unknown Slack action: {action}. Use: send_message, list_channels, list_messages, add_reaction, search_messages"
            ))),
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
}

async fn handle_slack_response(
    resp: Result<reqwest::Response, reqwest::Error>,
    success_msg: &str,
) -> Result<ToolResult> {
    match resp {
        Ok(r) if r.status().is_success() => {
            let body: serde_json::Value = r.json().await.unwrap_or_default();
            if body["ok"].as_bool().unwrap_or(false) {
                let ts = body["ts"]
                    .as_str()
                    .or_else(|| body["message"]["ts"].as_str())
                    .unwrap_or("ok");
                Ok(ToolResult::success(format!("{success_msg} (ts: {ts})")))
            } else {
                let err = body["error"].as_str().unwrap_or("unknown error");
                Ok(ToolResult::error(format!("Slack API error: {err}")))
            }
        }
        Ok(r) => {
            let status = r.status();
            let text = r.text().await.unwrap_or_default();
            Ok(ToolResult::error(format!("Slack API error {status}: {text}")))
        }
        Err(e) => Ok(ToolResult::error(format!("Request failed: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slack_tool_metadata() {
        let tool = SlackTool;
        assert_eq!(tool.name(), "Slack");
        assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    }

    #[test]
    fn test_slack_schema_has_action() {
        let tool = SlackTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("action")));

        let action_enum = schema["properties"]["action"]["enum"].as_array().unwrap();
        assert_eq!(action_enum.len(), 5);
        assert!(action_enum.contains(&json!("send_message")));
        assert!(action_enum.contains(&json!("list_channels")));
        assert!(action_enum.contains(&json!("list_messages")));
        assert!(action_enum.contains(&json!("add_reaction")));
        assert!(action_enum.contains(&json!("search_messages")));
    }

    #[tokio::test]
    async fn test_slack_missing_token() {
        std::env::remove_var("SLACK_TOKEN");

        let tool = SlackTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };
        let result = tool
            .call(json!({"action": "list_channels"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("SLACK_TOKEN"));
    }

    #[tokio::test]
    async fn test_slack_send_message_missing_channel() {
        std::env::set_var("SLACK_TOKEN", "xoxb-test");

        let tool = SlackTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };
        let result = tool
            .call(json!({"action": "send_message", "text": "hello"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("channel"));

        std::env::remove_var("SLACK_TOKEN");
    }

    #[tokio::test]
    async fn test_slack_send_message_missing_text() {
        std::env::set_var("SLACK_TOKEN", "xoxb-test");

        let tool = SlackTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };
        let result = tool
            .call(
                json!({"action": "send_message", "channel": "C123"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("text"));

        std::env::remove_var("SLACK_TOKEN");
    }

    #[tokio::test]
    async fn test_slack_list_messages_missing_channel() {
        std::env::set_var("SLACK_TOKEN", "xoxb-test");

        let tool = SlackTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };
        let result = tool
            .call(json!({"action": "list_messages"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("channel"));

        std::env::remove_var("SLACK_TOKEN");
    }

    #[tokio::test]
    async fn test_slack_add_reaction_missing_params() {
        std::env::set_var("SLACK_TOKEN", "xoxb-test");

        let tool = SlackTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };

        // Missing channel
        let result = tool
            .call(
                json!({"action": "add_reaction", "timestamp": "123", "emoji": "thumbsup"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("channel"));

        // Missing timestamp
        let result = tool
            .call(
                json!({"action": "add_reaction", "channel": "C123", "emoji": "thumbsup"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("timestamp"));

        // Missing emoji
        let result = tool
            .call(
                json!({"action": "add_reaction", "channel": "C123", "timestamp": "123"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("emoji"));

        std::env::remove_var("SLACK_TOKEN");
    }

    #[tokio::test]
    async fn test_slack_search_missing_query() {
        std::env::set_var("SLACK_TOKEN", "xoxb-test");

        let tool = SlackTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };
        let result = tool
            .call(json!({"action": "search_messages"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("query"));

        std::env::remove_var("SLACK_TOKEN");
    }

    #[tokio::test]
    async fn test_slack_unknown_action() {
        std::env::set_var("SLACK_TOKEN", "xoxb-test");

        let tool = SlackTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };
        let result = tool
            .call(json!({"action": "nuke_workspace"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("Unknown Slack action"));

        std::env::remove_var("SLACK_TOKEN");
    }
}
