//! Level 4: Multi-turn conversation tests.
//!
//! These verify that context is preserved across multiple submit_message calls.

use std::sync::Arc;
use std::time::Duration;
use cisco_code_tools::{read::ReadTool, write::WriteTool};
use cisco_code_e2e::skip_without_bedrock;
use cisco_code_e2e::helpers::provider::*;
use cisco_code_e2e::helpers::runtime::*;
use cisco_code_e2e::helpers::assertions::*;
use cisco_code_e2e::helpers::workspace::TestWorkspace;

#[tokio::test]
async fn test_context_preserved() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS * 2), async {
        let mut runtime = minimal_runtime(client);

        // Turn 1: Establish context
        let _events1 = runtime
            .submit_message("My name is Zephyr. Remember that.")
            .await
            .expect("turn 1 failed");

        // Turn 2: Recall context
        let events2 = runtime
            .submit_message("What is my name?")
            .await
            .expect("turn 2 failed");

        assert_text_contains(&events2, "Zephyr");
        assert_eq!(runtime.turn_count(), 2);

        // Session should have 4+ messages: user1, assistant1, user2, assistant2
        assert!(
            runtime.session.messages.len() >= 4,
            "Expected at least 4 messages, got {}",
            runtime.session.messages.len()
        );
    })
    .await;

    result.expect("test timed out");
}

#[tokio::test]
async fn test_file_ops_across_turns() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS * 2), async {
        let ws = TestWorkspace::new();

        let mut runtime = runtime_with_tools(client, &ws.path_str(), |tools| {
            tools.register(Arc::new(ReadTool)).unwrap();
            tools.register(Arc::new(WriteTool)).unwrap();
        });

        // Turn 1: Write a file
        let events1 = runtime
            .submit_message(r#"Write a file called config.json with the content: {"port": 8080}"#)
            .await
            .expect("turn 1 failed");

        assert_tool_called(&events1, "Write");
        assert!(ws.file_exists("config.json"), "config.json should exist after turn 1");

        // Turn 2: Read the file back
        let events2 = runtime
            .submit_message("Read config.json and tell me what port is configured.")
            .await
            .expect("turn 2 failed");

        assert_tool_called(&events2, "Read");
        assert_text_contains(&events2, "8080");
    })
    .await;

    result.expect("test timed out");
}

#[tokio::test]
async fn test_multi_turn_usage_tracking() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS * 2), async {
        let mut runtime = minimal_runtime(client);

        // Turn 1
        let _events1 = runtime
            .submit_message("Say hello.")
            .await
            .expect("turn 1 failed");
        let usage_1 = runtime.total_usage().total();
        assert!(usage_1 > 0);

        // Turn 2
        let _events2 = runtime
            .submit_message("Say goodbye.")
            .await
            .expect("turn 2 failed");
        let usage_2 = runtime.total_usage().total();
        assert!(usage_2 > usage_1, "Usage should grow: {} -> {}", usage_1, usage_2);

        assert_eq!(runtime.turn_count(), 2);
    })
    .await;

    result.expect("test timed out");
}
