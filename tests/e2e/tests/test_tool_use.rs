//! Level 3: Tool-use tests — model calls tools, results feed back.
//!
//! These exercise the full agent loop: model decides to call a tool,
//! the runtime executes it, feeds the result back, and the model responds.

use std::sync::Arc;
use std::time::Duration;
use cisco_code_tools::{read::ReadTool, write::WriteTool, bash::BashTool};
use cisco_code_e2e::skip_without_bedrock;
use cisco_code_e2e::helpers::provider::*;
use cisco_code_e2e::helpers::runtime::*;
use cisco_code_e2e::helpers::assertions::*;
use cisco_code_e2e::helpers::workspace::TestWorkspace;

#[tokio::test]
async fn test_read_file() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), async {
        let ws = TestWorkspace::new();
        ws.write_file("hello.txt", "The secret number is 42.");

        let mut runtime = runtime_with_tools(client, &ws.path_str(), |tools| {
            tools.register(Arc::new(ReadTool)).unwrap();
        });

        let events = runtime
            .submit_message("Read the file hello.txt and tell me what the secret number is.")
            .await
            .expect("submit_message failed");

        assert_tool_called(&events, "Read");
        assert_tool_succeeded(&events, "Read");
        assert_text_contains(&events, "42");
        assert!(count_turns(&events) >= 2, "Expected at least 2 turns (tool + response)");
    })
    .await;

    result.expect("test timed out");
}

#[tokio::test]
async fn test_write_file() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), async {
        let ws = TestWorkspace::new();

        let mut runtime = runtime_with_tools(client, &ws.path_str(), |tools| {
            tools.register(Arc::new(WriteTool)).unwrap();
        });

        let events = runtime
            .submit_message(
                "Create a file called greeting.txt with the content: Hello, World!",
            )
            .await
            .expect("submit_message failed");

        assert_tool_called(&events, "Write");

        // Verify the file was actually created on disk
        assert!(
            ws.file_exists("greeting.txt"),
            "Expected greeting.txt to be created"
        );
        let content = ws.read_file("greeting.txt");
        assert!(
            content.contains("Hello, World!"),
            "Expected file to contain 'Hello, World!', got: {content}"
        );
    })
    .await;

    result.expect("test timed out");
}

#[tokio::test]
async fn test_bash_command() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), async {
        let ws = TestWorkspace::new();
        ws.write_file("numbers.txt", "1\n2\n3\n4\n5\n");

        let mut runtime = runtime_with_tools(client, &ws.path_str(), |tools| {
            tools.register(Arc::new(BashTool)).unwrap();
        });

        let events = runtime
            .submit_message(
                "Use bash to count the number of lines in numbers.txt. Tell me the count.",
            )
            .await
            .expect("submit_message failed");

        assert_tool_called(&events, "Bash");
        assert_text_contains(&events, "5");
    })
    .await;

    result.expect("test timed out");
}

#[tokio::test]
async fn test_multi_tool_chain() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS * 2), async {
        let ws = TestWorkspace::new();
        ws.write_file("data.txt", "apple\nbanana\ncherry\ndate\n");

        let mut runtime = runtime_with_tools(client, &ws.path_str(), |tools| {
            tools.register(Arc::new(ReadTool)).unwrap();
            tools.register(Arc::new(BashTool)).unwrap();
        });

        let events = runtime
            .submit_message(
                "First read data.txt, then use bash to count how many lines contain the letter 'a'. Tell me the count.",
            )
            .await
            .expect("submit_message failed");

        // Should have used at least 2 tools
        assert!(
            count_turns(&events) >= 2,
            "Expected at least 2 turns for multi-tool chain"
        );

        // The text should contain the answer (apple, banana, date all have 'a' = 3)
        let text = collect_text(&events);
        assert!(
            text.contains('3') || text.contains("three"),
            "Expected answer to mention 3, got: {}",
            &text[..text.len().min(300)]
        );
    })
    .await;

    result.expect("test timed out");
}

#[tokio::test]
async fn test_tool_error_recovery() {
    let client = skip_without_bedrock!();

    let result = tokio::time::timeout(Duration::from_secs(TEST_TIMEOUT_SECS), async {
        let ws = TestWorkspace::new();
        // Empty workspace — no files to read

        let mut runtime = runtime_with_tools(client, &ws.path_str(), |tools| {
            tools.register(Arc::new(ReadTool)).unwrap();
        });

        let events = runtime
            .submit_message("Read the file nonexistent.txt")
            .await
            .expect("submit_message failed");

        assert_tool_called(&events, "Read");

        // Should have an error result
        let has_error = events.iter().any(|e| matches!(
            e,
            cisco_code_protocol::StreamEvent::ToolResult { is_error, .. } if *is_error
        ));
        assert!(has_error, "Expected a tool error result for nonexistent file");

        // Model should acknowledge the error in its response
        let text = collect_text(&events).to_lowercase();
        let acknowledges_error = text.contains("not found")
            || text.contains("doesn't exist")
            || text.contains("does not exist")
            || text.contains("no such file")
            || text.contains("error")
            || text.contains("couldn't");
        assert!(
            acknowledges_error,
            "Expected model to acknowledge the error, got: {}",
            &text[..text.len().min(300)]
        );
    })
    .await;

    result.expect("test timed out");
}
