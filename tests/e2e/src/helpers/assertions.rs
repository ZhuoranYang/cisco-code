//! Assertion helpers for E2E tests.
//!
//! These use structural assertions (tool called? error? stop reason?)
//! rather than exact string matching to handle LLM non-determinism.

use cisco_code_protocol::{StopReason, StreamEvent};

/// Collect all TextDelta strings into one combined response.
pub fn collect_text(events: &[StreamEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            StreamEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// Assert the combined text response contains a substring (case-insensitive).
pub fn assert_text_contains(events: &[StreamEvent], needle: &str) {
    let text = collect_text(events).to_lowercase();
    let needle_lower = needle.to_lowercase();
    assert!(
        text.contains(&needle_lower),
        "Expected response to contain '{}', got: '{}'",
        needle,
        &text[..text.len().min(200)]
    );
}

/// Assert that at least one TurnEnd event has the given stop reason.
pub fn assert_stop_reason(events: &[StreamEvent], expected: StopReason) {
    let found = events.iter().any(|e| matches!(
        e,
        StreamEvent::TurnEnd { stop_reason, .. } if *stop_reason == expected
    ));
    assert!(
        found,
        "Expected stop reason {:?}, not found in events",
        expected
    );
}

/// Assert a tool was called at least once with the given name.
pub fn assert_tool_called(events: &[StreamEvent], tool_name: &str) {
    let found = events.iter().any(|e| matches!(
        e,
        StreamEvent::ToolUseStart { tool_name: name, .. } if name == tool_name
    ));
    assert!(
        found,
        "Expected tool '{}' to be called, but it wasn't",
        tool_name
    );
}

/// Assert a ToolResult event exists that is not an error.
pub fn assert_tool_succeeded(events: &[StreamEvent], tool_name: &str) {
    assert_tool_called(events, tool_name);
    let has_success = events.iter().any(|e| matches!(
        e,
        StreamEvent::ToolResult { is_error, .. } if !is_error
    ));
    assert!(
        has_success,
        "Expected tool '{}' to succeed, but no successful result found",
        tool_name
    );
}

/// Assert that token usage is non-zero.
pub fn assert_nonzero_usage(events: &[StreamEvent]) {
    let has_usage = events.iter().any(|e| matches!(
        e,
        StreamEvent::TurnEnd { usage, .. } if usage.total() > 0
    ));
    assert!(has_usage, "Expected non-zero token usage");
}

/// Count the number of TurnStart events (== number of agent loop iterations).
pub fn count_turns(events: &[StreamEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, StreamEvent::TurnStart { .. }))
        .count()
}

/// Assert that no PermissionRequest events were emitted.
pub fn assert_no_permission_requests(events: &[StreamEvent]) {
    let count = events
        .iter()
        .filter(|e| matches!(e, StreamEvent::PermissionRequest { .. }))
        .count();
    assert_eq!(
        count, 0,
        "Expected no PermissionRequest events, found {}",
        count
    );
}

/// Assert events have correct structural ordering: TurnStart first, TurnEnd last.
pub fn assert_valid_event_ordering(events: &[StreamEvent]) {
    assert!(!events.is_empty(), "Expected at least one event");

    // First event should be TurnStart
    assert!(
        matches!(events.first(), Some(StreamEvent::TurnStart { .. })),
        "First event should be TurnStart, got: {:?}",
        events.first()
    );

    // Last event should be TurnEnd or Error
    assert!(
        matches!(
            events.last(),
            Some(StreamEvent::TurnEnd { .. }) | Some(StreamEvent::Error { .. })
        ),
        "Last event should be TurnEnd or Error, got: {:?}",
        events.last()
    );
}
