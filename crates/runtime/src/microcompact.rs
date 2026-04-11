//! Micro-compaction: lightweight context size reduction without LLM summarization.
//!
//! Design insight from Claude Code: Before triggering expensive full compaction,
//! aggressively clear old tool results (FRC — Function Result Clearing). This
//! reduces context size by 30-50% with zero API cost.
//!
//! Strategy tiers:
//! 1. **Micro** (50% threshold): Clear old tool results, keep last N intact
//! 2. **Full** (80% threshold): LLM summarization (handled by `Compactor`)
//! 3. **Emergency** (90% threshold): Aggressive compaction with fewer preserved messages

use cisco_code_protocol::Message;

/// Configuration for micro-compaction.
#[derive(Debug, Clone)]
pub struct MicroCompactConfig {
    /// Number of most recent tool results to keep intact.
    pub results_to_keep: usize,
    /// Maximum characters per tool result (older results are truncated to this).
    pub max_chars_per_result: usize,
    /// Placeholder text for cleared results.
    pub cleared_placeholder: String,
}

impl Default for MicroCompactConfig {
    fn default() -> Self {
        Self {
            results_to_keep: 5,
            max_chars_per_result: 2000,
            cleared_placeholder: "[result cleared — see summary above]".to_string(),
        }
    }
}

/// Lightweight compactor that reduces context size without LLM calls.
pub struct MicroCompactor {
    config: MicroCompactConfig,
}

impl MicroCompactor {
    pub fn new(config: MicroCompactConfig) -> Self {
        Self { config }
    }

    /// Clear old tool results, keeping only the most recent N intact.
    ///
    /// Returns the number of results that were cleared.
    pub fn clear_old_results(&self, messages: &mut Vec<Message>) -> usize {
        // Count tool results from the end to find which ones to keep
        let mut result_indices: Vec<usize> = Vec::new();
        for (i, msg) in messages.iter().enumerate() {
            if matches!(msg, Message::ToolResult(_)) {
                result_indices.push(i);
            }
        }

        if result_indices.len() <= self.config.results_to_keep {
            return 0; // Nothing to clear
        }

        // Clear all but the most recent N
        let clear_count = result_indices.len() - self.config.results_to_keep;
        let indices_to_clear = &result_indices[..clear_count];

        let mut cleared = 0;
        for &idx in indices_to_clear {
            if let Message::ToolResult(ref mut result) = messages[idx] {
                if result.content != self.config.cleared_placeholder {
                    result.content = self.config.cleared_placeholder.clone();
                    cleared += 1;
                }
            }
        }

        cleared
    }

    /// Truncate large tool results to max_chars_per_result.
    ///
    /// Returns the number of results that were truncated.
    pub fn truncate_results(&self, messages: &mut Vec<Message>) -> usize {
        let mut truncated = 0;

        for msg in messages.iter_mut() {
            if let Message::ToolResult(result) = msg {
                if result.content.len() > self.config.max_chars_per_result
                    && result.content != self.config.cleared_placeholder
                {
                    // Find a safe byte boundary that doesn't split UTF-8 chars
                    let mut truncation_point = self.config.max_chars_per_result;
                    while truncation_point > 0
                        && !result.content.is_char_boundary(truncation_point)
                    {
                        truncation_point -= 1;
                    }
                    // Try to truncate at a line boundary for cleaner output
                    let cut_at = result.content[..truncation_point]
                        .rfind('\n')
                        .unwrap_or(truncation_point);
                    result.content = format!(
                        "{}\n\n[... truncated {} chars]",
                        &result.content[..cut_at],
                        result.content.len() - cut_at
                    );
                    truncated += 1;
                }
            }
        }

        truncated
    }

    /// Run both clearing and truncation.
    /// Returns (cleared_count, truncated_count).
    pub fn run(&self, messages: &mut Vec<Message>) -> (usize, usize) {
        let cleared = self.clear_old_results(messages);
        let truncated = self.truncate_results(messages);
        if cleared > 0 || truncated > 0 {
            tracing::info!(
                "Micro-compaction: cleared {} old results, truncated {} large results",
                cleared,
                truncated
            );
        }
        (cleared, truncated)
    }
}

/// Compaction level determined by context usage ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionLevel {
    /// No compaction needed.
    None,
    /// Micro-compaction: clear old tool results (50% of threshold).
    Micro,
    /// Full LLM summarization (80% of threshold).
    Full,
    /// Emergency: aggressive compaction with fewer preserved messages (90%).
    Emergency,
}

/// Determine the compaction level based on estimated tokens and threshold.
pub fn compaction_level(estimated_tokens: u64, threshold: u64) -> CompactionLevel {
    if threshold == 0 {
        return if estimated_tokens > 0 {
            CompactionLevel::Emergency
        } else {
            CompactionLevel::None
        };
    }
    let ratio = estimated_tokens as f64 / threshold as f64;
    if ratio >= 0.90 {
        CompactionLevel::Emergency
    } else if ratio >= 0.80 {
        CompactionLevel::Full
    } else if ratio >= 0.50 {
        CompactionLevel::Micro
    } else {
        CompactionLevel::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cisco_code_protocol::{ContentBlock, ToolResultMessage, UserMessage};
    use uuid::Uuid;

    fn make_user_msg(text: &str) -> Message {
        Message::User(UserMessage {
            id: Uuid::new_v4(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            attachments: None,
        })
    }

    fn make_tool_result(content: &str) -> Message {
        Message::ToolResult(ToolResultMessage {
            id: Uuid::new_v4(),
            tool_use_id: format!("tu_{}", Uuid::new_v4()),
            content: content.to_string(),
            is_error: false,
            injected_messages: None,
        })
    }

    #[test]
    fn test_clear_old_results_keeps_recent() {
        let config = MicroCompactConfig {
            results_to_keep: 2,
            ..Default::default()
        };
        let mc = MicroCompactor::new(config);

        let mut messages = vec![
            make_user_msg("q1"),
            make_tool_result("result 1"),
            make_user_msg("q2"),
            make_tool_result("result 2"),
            make_user_msg("q3"),
            make_tool_result("result 3"),
            make_user_msg("q4"),
            make_tool_result("result 4"),
        ];

        let cleared = mc.clear_old_results(&mut messages);
        assert_eq!(cleared, 2); // results 1 and 2 cleared

        // Results 1 and 2 should be cleared
        if let Message::ToolResult(ref r) = messages[1] {
            assert!(r.content.contains("cleared"));
        }
        if let Message::ToolResult(ref r) = messages[3] {
            assert!(r.content.contains("cleared"));
        }

        // Results 3 and 4 should be intact
        if let Message::ToolResult(ref r) = messages[5] {
            assert_eq!(r.content, "result 3");
        }
        if let Message::ToolResult(ref r) = messages[7] {
            assert_eq!(r.content, "result 4");
        }
    }

    #[test]
    fn test_clear_old_results_noop_when_few() {
        let mc = MicroCompactor::new(MicroCompactConfig::default());
        let mut messages = vec![
            make_user_msg("q1"),
            make_tool_result("result 1"),
        ];

        let cleared = mc.clear_old_results(&mut messages);
        assert_eq!(cleared, 0);
    }

    #[test]
    fn test_truncate_results() {
        let config = MicroCompactConfig {
            max_chars_per_result: 50,
            ..Default::default()
        };
        let mc = MicroCompactor::new(config);

        let long_content = "x".repeat(200);
        let mut messages = vec![
            make_tool_result(&long_content),
            make_tool_result("short"),
        ];

        let truncated = mc.truncate_results(&mut messages);
        assert_eq!(truncated, 1);

        if let Message::ToolResult(ref r) = messages[0] {
            assert!(r.content.len() < 200);
            assert!(r.content.contains("truncated"));
        }
        if let Message::ToolResult(ref r) = messages[1] {
            assert_eq!(r.content, "short");
        }
    }

    #[test]
    fn test_run_both() {
        let config = MicroCompactConfig {
            results_to_keep: 1,
            max_chars_per_result: 20,
            ..Default::default()
        };
        let mc = MicroCompactor::new(config);

        let mut messages = vec![
            make_tool_result("old result that is longer than limit"),
            make_tool_result("another old result"),
            make_tool_result("recent result"),
        ];

        let (cleared, _truncated) = mc.run(&mut messages);
        assert_eq!(cleared, 2); // First two cleared
    }

    #[test]
    fn test_compaction_level_none() {
        assert_eq!(compaction_level(30_000, 100_000), CompactionLevel::None);
    }

    #[test]
    fn test_compaction_level_micro() {
        assert_eq!(compaction_level(55_000, 100_000), CompactionLevel::Micro);
    }

    #[test]
    fn test_compaction_level_full() {
        assert_eq!(compaction_level(85_000, 100_000), CompactionLevel::Full);
    }

    #[test]
    fn test_compaction_level_emergency() {
        assert_eq!(compaction_level(95_000, 100_000), CompactionLevel::Emergency);
    }

    #[test]
    fn test_compaction_level_zero_threshold() {
        // Division by zero guard
        assert_eq!(compaction_level(1000, 0), CompactionLevel::Emergency);
        assert_eq!(compaction_level(0, 0), CompactionLevel::None);
    }

    #[test]
    fn test_truncate_multibyte_utf8() {
        let config = MicroCompactConfig {
            max_chars_per_result: 10,
            ..Default::default()
        };
        let mc = MicroCompactor::new(config);

        // 🎉 is 4 bytes in UTF-8; truncation at byte 10 could split a char
        let mut messages = vec![make_tool_result("Hello 🎉🎉🎉 world!")];
        let truncated = mc.truncate_results(&mut messages);
        assert_eq!(truncated, 1);

        // Should not panic and content should be valid UTF-8
        if let Message::ToolResult(ref r) = messages[0] {
            assert!(r.content.is_char_boundary(0)); // valid string
            assert!(r.content.contains("truncated"));
        }
    }

    #[test]
    fn test_already_cleared_not_recounted() {
        let config = MicroCompactConfig {
            results_to_keep: 1,
            ..Default::default()
        };
        let mc = MicroCompactor::new(config);

        let mut messages = vec![
            make_tool_result("[result cleared — see summary above]"),
            make_tool_result("recent"),
        ];

        let cleared = mc.clear_old_results(&mut messages);
        assert_eq!(cleared, 0); // Already cleared, should not count
    }
}
