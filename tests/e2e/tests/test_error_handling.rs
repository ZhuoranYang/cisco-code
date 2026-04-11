//! Level 5: Error handling and edge case tests.
//!
//! These test limits, permission modes, and event ordering invariants.

use std::sync::Arc;
use std::time::Duration;
use cisco_code_tools::bash::BashTool;
use cisco_code_e2e::skip_without_bedrock;
use cisco_code_e2e::helpers::provider::*;
use cisco_code_e2e::helpers::runtime::*;
use cisco_code_e2e::helpers::assertions::*;

#[tokio::test]
async fn test_max_turns_limit() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS * 2), async {
        // Runtime with max_turns=2 and a tool that encourages looping
        let mut runtime = runtime_with_max_turns(client, 2, |tools| {
            tools.register(Arc::new(BashTool)).unwrap();
        });

        let events = runtime
            .submit_message(
                "Run these bash commands one at a time: echo step1, echo step2, echo step3, echo step4. Report all outputs.",
            )
            .await
            .expect("submit_message failed");

        // Should not exceed max_turns
        let turns = count_turns(&events);
        assert!(
            turns <= 3, // max_turns=2 means up to 2 tool turns + 1 final
            "Expected at most 3 turns with max_turns=2, got {}",
            turns
        );
    })
    .await;

    result.expect("test timed out");
}

#[tokio::test]
async fn test_bypass_no_permission_prompts() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), async {
        let mut runtime = runtime_with_tools(client, "/tmp", |tools| {
            tools.register(Arc::new(BashTool)).unwrap();
        });

        let events = runtime
            .submit_message("Run the command: echo test")
            .await
            .expect("submit_message failed");

        // In bypass mode, there should be no permission requests
        assert_no_permission_requests(&events);
        assert_tool_called(&events, "Bash");
    })
    .await;

    result.expect("test timed out");
}

#[tokio::test]
async fn test_event_ordering() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), async {
        let mut runtime = minimal_runtime(client);

        let events = runtime
            .submit_message("Hi")
            .await
            .expect("submit_message failed");

        assert_valid_event_ordering(&events);

        // Verify turn numbers are monotonically increasing
        let turn_numbers: Vec<u32> = events
            .iter()
            .filter_map(|e| match e {
                cisco_code_protocol::StreamEvent::TurnStart { turn_number, .. } => {
                    Some(*turn_number)
                }
                _ => None,
            })
            .collect();

        for window in turn_numbers.windows(2) {
            assert!(
                window[1] > window[0],
                "Turn numbers should be monotonically increasing: {:?}",
                turn_numbers
            );
        }
    })
    .await;

    result.expect("test timed out");
}
