//! Level 1: Direct BedrockClient.stream() API tests.
//!
//! These test the raw provider without the ConversationRuntime wrapper.

use std::time::Duration;
use cisco_code_api::{AssistantEvent, CompletionRequest, Provider};
use cisco_code_protocol::ToolDefinition;
use cisco_code_e2e::skip_without_bedrock;
use cisco_code_e2e::helpers::provider::*;

#[tokio::test]
async fn test_simple_text() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), async {
        let request = CompletionRequest {
            model: HAIKU_MODEL.to_string(),
            system_prompt: "You are a helpful assistant. Be concise.".to_string(),
            messages: vec![cisco_code_api::ApiMessage {
                role: "user".to_string(),
                content: serde_json::json!("What is 2+2? Answer with just the number."),
            }],
            tools: vec![],
            max_tokens: E2E_MAX_TOKENS,
            temperature: Some(E2E_TEMPERATURE),
            thinking: None,
            system_blocks: None,
        };

        let events = client.stream(request).await.expect("stream failed");

        // Should have at least one text delta containing "4"
        let text: String = events
            .iter()
            .filter_map(|e| match e {
                AssistantEvent::TextDelta(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert!(text.contains('4'), "Expected '4' in response, got: {text}");

        // Should have a MessageStop with end_turn
        let has_stop = events.iter().any(|e| matches!(
            e,
            AssistantEvent::MessageStop { stop_reason } if stop_reason == "end_turn"
        ));
        assert!(has_stop, "Expected end_turn stop reason");

        // Should have usage data
        let has_usage = events.iter().any(|e| matches!(
            e,
            AssistantEvent::Usage { input_tokens, output_tokens }
                if *input_tokens > 0 && *output_tokens > 0
        ));
        assert!(has_usage, "Expected non-zero usage");
    })
    .await;

    result.expect("test timed out");
}

#[tokio::test]
async fn test_tool_use_request() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), async {
        let request = CompletionRequest {
            model: HAIKU_MODEL.to_string(),
            system_prompt: "You are a helpful assistant. When asked about weather, always use the get_weather tool.".to_string(),
            messages: vec![cisco_code_api::ApiMessage {
                role: "user".to_string(),
                content: serde_json::json!("What is the weather in San Francisco? Use the get_weather tool."),
            }],
            tools: vec![ToolDefinition {
                name: "get_weather".to_string(),
                description: "Get the current weather for a city.".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "city": { "type": "string", "description": "City name" }
                    },
                    "required": ["city"]
                }),
            }],
            max_tokens: E2E_MAX_TOKENS,
            temperature: Some(E2E_TEMPERATURE),
            thinking: None,
            system_blocks: None,
        };

        let events = client.stream(request).await.expect("stream failed");

        // Should have a tool use event
        let tool_use = events.iter().find(|e| matches!(e, AssistantEvent::ToolUse { .. }));
        assert!(tool_use.is_some(), "Expected a ToolUse event");

        if let Some(AssistantEvent::ToolUse { name, input, .. }) = tool_use {
            assert_eq!(name, "get_weather");
            assert!(input.get("city").is_some(), "Expected 'city' in tool input");
        }

        // Stop reason should be tool_use
        let has_tool_stop = events.iter().any(|e| matches!(
            e,
            AssistantEvent::MessageStop { stop_reason } if stop_reason == "tool_use"
        ));
        assert!(has_tool_stop, "Expected tool_use stop reason");
    })
    .await;

    result.expect("test timed out");
}

#[tokio::test]
async fn test_max_tokens_respected() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), async {
        let request = CompletionRequest {
            model: HAIKU_MODEL.to_string(),
            system_prompt: "You are a helpful assistant.".to_string(),
            messages: vec![cisco_code_api::ApiMessage {
                role: "user".to_string(),
                content: serde_json::json!("Write a 500-word essay about the history of computing."),
            }],
            tools: vec![],
            max_tokens: 10, // Very low limit
            temperature: Some(E2E_TEMPERATURE),
            thinking: None,
            system_blocks: None,
        };

        let events = client.stream(request).await.expect("stream failed");

        // Should stop due to max_tokens
        let has_max_stop = events.iter().any(|e| matches!(
            e,
            AssistantEvent::MessageStop { stop_reason } if stop_reason == "max_tokens"
        ));
        assert!(has_max_stop, "Expected max_tokens stop reason");
    })
    .await;

    result.expect("test timed out");
}

#[tokio::test]
async fn test_no_tools_no_tool_call() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), async {
        let request = CompletionRequest {
            model: HAIKU_MODEL.to_string(),
            system_prompt: "You are a helpful assistant. Be concise.".to_string(),
            messages: vec![cisco_code_api::ApiMessage {
                role: "user".to_string(),
                content: serde_json::json!("Explain what Rust's ownership system does in one sentence."),
            }],
            tools: vec![],
            max_tokens: E2E_MAX_TOKENS,
            temperature: Some(E2E_TEMPERATURE),
            thinking: None,
            system_blocks: None,
        };

        let events = client.stream(request).await.expect("stream failed");

        // Should NOT have any tool use events
        let tool_uses: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, AssistantEvent::ToolUse { .. }))
            .collect();
        assert!(
            tool_uses.is_empty(),
            "Expected no tool use events, got {}",
            tool_uses.len()
        );

        // Should have text and end_turn
        let has_text = events.iter().any(|e| matches!(e, AssistantEvent::TextDelta(_)));
        assert!(has_text, "Expected text in response");

        let has_end = events.iter().any(|e| matches!(
            e,
            AssistantEvent::MessageStop { stop_reason } if stop_reason == "end_turn"
        ));
        assert!(has_end, "Expected end_turn stop reason");
    })
    .await;

    result.expect("test timed out");
}
