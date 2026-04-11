//! Level 2: ConversationRuntime text-only turn tests.
//!
//! These test the runtime wrapper around the provider without tool use.

use std::time::Duration;
use cisco_code_protocol::StopReason;
use cisco_code_e2e::skip_without_bedrock;
use cisco_code_e2e::helpers::provider::*;
use cisco_code_e2e::helpers::runtime::*;
use cisco_code_e2e::helpers::assertions::*;

#[tokio::test]
async fn test_simple_turn() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), async {
        let mut runtime = minimal_runtime(client);

        let events = runtime
            .submit_message("What is the capital of France? Answer in one word.")
            .await
            .expect("submit_message failed");

        assert_text_contains(&events, "Paris");
        assert_stop_reason(&events, StopReason::EndTurn);
        assert_nonzero_usage(&events);
        assert_eq!(count_turns(&events), 1);
        assert_valid_event_ordering(&events);
    })
    .await;

    result.expect("test timed out");
}

#[tokio::test]
async fn test_session_messages() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), async {
        let mut runtime = minimal_runtime(client);

        let _events = runtime
            .submit_message("Say hello.")
            .await
            .expect("submit_message failed");

        // Session should have accumulated messages
        assert!(
            runtime.session.messages.len() >= 2,
            "Expected at least 2 messages (user + assistant), got {}",
            runtime.session.messages.len()
        );

        assert_eq!(runtime.turn_count(), 1);
    })
    .await;

    result.expect("test timed out");
}

#[tokio::test]
async fn test_usage_accumulates_across_turns() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS * 2), async {
        let mut runtime = minimal_runtime(client);

        // Turn 1
        let _events1 = runtime
            .submit_message("Say hi.")
            .await
            .expect("turn 1 failed");
        let usage_after_1 = runtime.total_usage().total();
        assert!(usage_after_1 > 0, "Expected non-zero usage after turn 1");

        // Turn 2
        let _events2 = runtime
            .submit_message("Say bye.")
            .await
            .expect("turn 2 failed");
        let usage_after_2 = runtime.total_usage().total();
        assert!(
            usage_after_2 > usage_after_1,
            "Expected usage to increase: {} -> {}",
            usage_after_1,
            usage_after_2
        );

        assert_eq!(runtime.turn_count(), 2);
    })
    .await;

    result.expect("test timed out");
}
