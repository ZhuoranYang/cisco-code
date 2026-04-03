//! Webex tool — interact with Cisco Webex Teams via REST API.
//!
//! Supports:
//! - send_message: Send a message to a Webex room or person
//! - list_rooms: List rooms the bot/user is in
//! - list_messages: List recent messages in a room
//! - create_room: Create a new Webex room
//! - get_person: Look up a person by email
//!
//! Auth: WEBEX_TOKEN environment variable (Bearer token).
//! API docs: https://developer.webex.com/docs/api/getting-started

use anyhow::Result;
use cisco_code_protocol::{PermissionLevel, ToolResult};
use serde_json::json;

use crate::{Tool, ToolContext};

const WEBEX_BASE_URL: &str = "https://webexapis.com/v1";

pub struct WebexTool;

impl WebexTool {
    fn get_token() -> Result<String> {
        std::env::var("WEBEX_TOKEN")
            .map_err(|_| anyhow::anyhow!("WEBEX_TOKEN not set. Set it to your Webex Bot or Personal Access Token."))
    }

    fn http_client() -> reqwest::Client {
        reqwest::Client::new()
    }
}

impl Tool for WebexTool {
    fn name(&self) -> &str {
        "Webex"
    }

    fn description(&self) -> &str {
        "Interact with Cisco Webex Teams. Actions: send_message, list_rooms, list_messages, create_room, get_person. Requires WEBEX_TOKEN env var."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["send_message", "list_rooms", "list_messages", "create_room", "get_person"],
                    "description": "The Webex action to perform"
                },
                "room_id": {
                    "type": "string",
                    "description": "Room ID (for send_message, list_messages)"
                },
                "person_email": {
                    "type": "string",
                    "description": "Email address (for send_message to a person, get_person)"
                },
                "text": {
                    "type": "string",
                    "description": "Message text (for send_message)"
                },
                "markdown": {
                    "type": "string",
                    "description": "Message in markdown format (for send_message, optional)"
                },
                "room_title": {
                    "type": "string",
                    "description": "Room title (for create_room)"
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
                let mut body = serde_json::Map::new();

                if let Some(room_id) = input["room_id"].as_str() {
                    body.insert("roomId".into(), json!(room_id));
                } else if let Some(email) = input["person_email"].as_str() {
                    body.insert("toPersonEmail".into(), json!(email));
                } else {
                    return Ok(ToolResult::error(
                        "send_message requires either room_id or person_email",
                    ));
                }

                if let Some(markdown) = input["markdown"].as_str() {
                    body.insert("markdown".into(), json!(markdown));
                } else if let Some(text) = input["text"].as_str() {
                    body.insert("text".into(), json!(text));
                } else {
                    return Ok(ToolResult::error("send_message requires text or markdown"));
                }

                let resp = client
                    .post(format!("{WEBEX_BASE_URL}/messages"))
                    .bearer_auth(&token)
                    .json(&body)
                    .send()
                    .await;

                handle_response(resp, "Message sent").await
            }

            "list_rooms" => {
                let resp = client
                    .get(format!("{WEBEX_BASE_URL}/rooms"))
                    .bearer_auth(&token)
                    .query(&[("max", max_results.to_string())])
                    .send()
                    .await;

                match resp {
                    Ok(r) if r.status().is_success() => {
                        let body: serde_json::Value = r.json().await.unwrap_or_default();
                        let rooms = body["items"].as_array();
                        match rooms {
                            Some(items) => {
                                let summary: Vec<String> = items
                                    .iter()
                                    .map(|room| {
                                        format!(
                                            "- {} (id: {})",
                                            room["title"].as_str().unwrap_or("untitled"),
                                            room["id"].as_str().unwrap_or("?")
                                        )
                                    })
                                    .collect();
                                Ok(ToolResult::success(format!(
                                    "{} room(s):\n{}",
                                    summary.len(),
                                    summary.join("\n")
                                )))
                            }
                            None => Ok(ToolResult::success("No rooms found.")),
                        }
                    }
                    Ok(r) => {
                        let status = r.status();
                        let text = r.text().await.unwrap_or_default();
                        Ok(ToolResult::error(format!("Webex API error {status}: {text}")))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Request failed: {e}"))),
                }
            }

            "list_messages" => {
                let room_id = match input["room_id"].as_str() {
                    Some(id) => id,
                    None => return Ok(ToolResult::error("list_messages requires room_id")),
                };

                let resp = client
                    .get(format!("{WEBEX_BASE_URL}/messages"))
                    .bearer_auth(&token)
                    .query(&[
                        ("roomId", room_id.to_string()),
                        ("max", max_results.to_string()),
                    ])
                    .send()
                    .await;

                match resp {
                    Ok(r) if r.status().is_success() => {
                        let body: serde_json::Value = r.json().await.unwrap_or_default();
                        let messages = body["items"].as_array();
                        match messages {
                            Some(items) => {
                                let summary: Vec<String> = items
                                    .iter()
                                    .map(|msg| {
                                        format!(
                                            "[{}] {}: {}",
                                            msg["created"].as_str().unwrap_or("?"),
                                            msg["personEmail"].as_str().unwrap_or("unknown"),
                                            msg["text"]
                                                .as_str()
                                                .unwrap_or("")
                                                .chars()
                                                .take(200)
                                                .collect::<String>()
                                        )
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
                        Ok(ToolResult::error(format!("Webex API error {status}: {text}")))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Request failed: {e}"))),
                }
            }

            "create_room" => {
                let title = match input["room_title"].as_str() {
                    Some(t) => t,
                    None => return Ok(ToolResult::error("create_room requires room_title")),
                };

                let resp = client
                    .post(format!("{WEBEX_BASE_URL}/rooms"))
                    .bearer_auth(&token)
                    .json(&json!({"title": title}))
                    .send()
                    .await;

                handle_response(resp, &format!("Room '{title}' created")).await
            }

            "get_person" => {
                let email = match input["person_email"].as_str() {
                    Some(e) => e,
                    None => return Ok(ToolResult::error("get_person requires person_email")),
                };

                let resp = client
                    .get(format!("{WEBEX_BASE_URL}/people"))
                    .bearer_auth(&token)
                    .query(&[("email", email)])
                    .send()
                    .await;

                match resp {
                    Ok(r) if r.status().is_success() => {
                        let body: serde_json::Value = r.json().await.unwrap_or_default();
                        let people = body["items"].as_array();
                        match people {
                            Some(items) if !items.is_empty() => {
                                let person = &items[0];
                                Ok(ToolResult::success(format!(
                                    "Name: {}\nEmail: {}\nOrg: {}\nID: {}",
                                    person["displayName"].as_str().unwrap_or("?"),
                                    person["emails"][0].as_str().unwrap_or("?"),
                                    person["orgId"].as_str().unwrap_or("?"),
                                    person["id"].as_str().unwrap_or("?"),
                                )))
                            }
                            _ => Ok(ToolResult::success(format!(
                                "No person found with email: {email}"
                            ))),
                        }
                    }
                    Ok(r) => {
                        let status = r.status();
                        let text = r.text().await.unwrap_or_default();
                        Ok(ToolResult::error(format!("Webex API error {status}: {text}")))
                    }
                    Err(e) => Ok(ToolResult::error(format!("Request failed: {e}"))),
                }
            }

            _ => Ok(ToolResult::error(format!(
                "Unknown Webex action: {action}. Use: send_message, list_rooms, list_messages, create_room, get_person"
            ))),
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }
}

async fn handle_response(
    resp: Result<reqwest::Response, reqwest::Error>,
    success_msg: &str,
) -> Result<ToolResult> {
    match resp {
        Ok(r) if r.status().is_success() => {
            let body: serde_json::Value = r.json().await.unwrap_or_default();
            let id = body["id"].as_str().unwrap_or("unknown");
            Ok(ToolResult::success(format!("{success_msg} (id: {id})")))
        }
        Ok(r) => {
            let status = r.status();
            let text = r.text().await.unwrap_or_default();
            Ok(ToolResult::error(format!("Webex API error {status}: {text}")))
        }
        Err(e) => Ok(ToolResult::error(format!("Request failed: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webex_tool_metadata() {
        let tool = WebexTool;
        assert_eq!(tool.name(), "Webex");
        assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    }

    #[test]
    fn test_webex_schema_has_action() {
        let tool = WebexTool;
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("action")));

        let action_enum = schema["properties"]["action"]["enum"].as_array().unwrap();
        assert_eq!(action_enum.len(), 5);
        assert!(action_enum.contains(&json!("send_message")));
        assert!(action_enum.contains(&json!("list_rooms")));
        assert!(action_enum.contains(&json!("list_messages")));
        assert!(action_enum.contains(&json!("create_room")));
        assert!(action_enum.contains(&json!("get_person")));
    }

    #[tokio::test]
    async fn test_webex_missing_token() {
        // Ensure WEBEX_TOKEN is not set for this test
        std::env::remove_var("WEBEX_TOKEN");

        let tool = WebexTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };
        let result = tool
            .call(json!({"action": "list_rooms"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("WEBEX_TOKEN"));
    }

    #[tokio::test]
    async fn test_webex_send_message_missing_target() {
        std::env::set_var("WEBEX_TOKEN", "test-token");

        let tool = WebexTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };
        let result = tool
            .call(json!({"action": "send_message", "text": "hello"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("room_id or person_email"));

        std::env::remove_var("WEBEX_TOKEN");
    }

    #[tokio::test]
    async fn test_webex_send_message_missing_text() {
        std::env::set_var("WEBEX_TOKEN", "test-token");

        let tool = WebexTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };
        let result = tool
            .call(
                json!({"action": "send_message", "room_id": "room123"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("text or markdown"));

        std::env::remove_var("WEBEX_TOKEN");
    }

    #[tokio::test]
    async fn test_webex_list_messages_missing_room() {
        std::env::set_var("WEBEX_TOKEN", "test-token");

        let tool = WebexTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };
        let result = tool
            .call(json!({"action": "list_messages"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("room_id"));

        std::env::remove_var("WEBEX_TOKEN");
    }

    #[tokio::test]
    async fn test_webex_create_room_missing_title() {
        std::env::set_var("WEBEX_TOKEN", "test-token");

        let tool = WebexTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };
        let result = tool
            .call(json!({"action": "create_room"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("room_title"));

        std::env::remove_var("WEBEX_TOKEN");
    }

    #[tokio::test]
    async fn test_webex_get_person_missing_email() {
        std::env::set_var("WEBEX_TOKEN", "test-token");

        let tool = WebexTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };
        let result = tool
            .call(json!({"action": "get_person"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("person_email"));

        std::env::remove_var("WEBEX_TOKEN");
    }

    #[tokio::test]
    async fn test_webex_unknown_action() {
        std::env::set_var("WEBEX_TOKEN", "test-token");

        let tool = WebexTool;
        let ctx = crate::ToolContext {
            cwd: "/tmp".into(),
            interactive: false,
        };
        let result = tool
            .call(json!({"action": "delete_everything"}), &ctx)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output.contains("Unknown Webex action"));

        std::env::remove_var("WEBEX_TOKEN");
    }
}
