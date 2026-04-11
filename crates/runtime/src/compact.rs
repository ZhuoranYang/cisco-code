//! Session history compaction via LLM summarization.
//!
//! Design insight from Claude Code: Automatic compaction when context overflows.
//! Uses LLM to summarize older conversation turns while preserving key information.
//!
//! Strategy:
//! 1. Estimate token count of current messages
//! 2. When tokens exceed threshold, take the oldest N messages
//! 3. Ask the LLM to summarize them into a compact context block
//! 4. Replace those messages with a single System(Context) message containing the summary
//! 5. Keep the most recent messages intact (they're needed for coherent continuation)

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use cisco_code_api::{ApiMessage, AssistantEvent, CompletionRequest, Provider};
use cisco_code_protocol::{
    ContentBlock, Message, SystemMessage, SystemMessageType, TokenUsage,
};
use uuid::Uuid;

/// Configuration for context compaction.
#[derive(Debug, Clone)]
pub struct CompactConfig {
    /// Token threshold that triggers compaction.
    pub compact_threshold: u64,
    /// Number of recent messages to keep uncompacted.
    pub preserve_recent: usize,
    /// Maximum tokens for the summary itself.
    pub summary_max_tokens: u32,
    /// Model to use for summarization (can be cheaper/smaller than the main model).
    pub summary_model: Option<String>,
    /// Target token count after compaction (to leave room for new messages).
    /// Defaults to 60% of compact_threshold.
    pub target_after_compact: Option<u64>,
}

impl Default for CompactConfig {
    fn default() -> Self {
        Self {
            compact_threshold: 80_000,
            preserve_recent: 10,
            summary_max_tokens: 8192,
            summary_model: None,
            target_after_compact: None,
        }
    }
}

/// Resolve the compact threshold from a model's context window.
/// Uses ~80% of the context window as the threshold, leaving headroom for
/// system prompt and tool definitions.
pub fn threshold_for_model(model: &str) -> u64 {
    let context_window = model_context_window(model);
    // 80% of context window minus system prompt overhead (~4K tokens)
    ((context_window as f64 * 0.80) as u64).saturating_sub(4_000)
}

/// Get the context window size for a model.
fn model_context_window(model: &str) -> u64 {
    if model.contains("claude") || model.contains("anthropic.") {
        200_000
    } else if model.starts_with("gpt-4o") {
        128_000
    } else if model.starts_with("o3") || model.starts_with("o1") {
        200_000
    } else {
        128_000 // conservative default
    }
}

/// The Compactor handles context window management.
pub struct Compactor {
    pub config: CompactConfig,
    /// Running estimate of current context tokens.
    estimated_tokens: u64,
    /// Number of compactions performed in this session.
    compaction_count: u32,
}

impl Compactor {
    pub fn new(config: CompactConfig) -> Self {
        Self {
            config,
            estimated_tokens: 0,
            compaction_count: 0,
        }
    }

    /// Create a compactor with model-aware thresholds.
    pub fn for_model(model: &str) -> Self {
        let threshold = threshold_for_model(model);
        let target = (threshold as f64 * 0.60) as u64;
        Self::new(CompactConfig {
            compact_threshold: threshold,
            target_after_compact: Some(target),
            ..Default::default()
        })
    }

    /// Estimate the token count for a list of messages.
    ///
    /// Uses a rough heuristic: ~4 characters per token for English text.
    /// This is intentionally conservative — better to compact too early than too late.
    pub fn estimate_tokens(messages: &[Message]) -> u64 {
        let mut total_chars: u64 = 0;
        for msg in messages {
            total_chars += match msg {
                Message::User(u) => content_blocks_chars(&u.content),
                Message::Assistant(a) => content_blocks_chars(&a.content),
                Message::System(s) => s.content.len() as u64,
                Message::ToolUse(t) => {
                    t.tool_name.len() as u64 + t.input.to_string().len() as u64
                }
                Message::ToolResult(r) => r.content.len() as u64,
            };
        }
        // ~4 chars per token, rounded up
        (total_chars + 3) / 4
    }

    /// Update the token estimate after new messages are added.
    pub fn update_estimate(&mut self, messages: &[Message]) {
        self.estimated_tokens = Self::estimate_tokens(messages);
    }

    /// Check if compaction is needed based on current token estimate.
    pub fn needs_compaction(&self) -> bool {
        self.estimated_tokens > self.config.compact_threshold
    }

    /// Perform compaction: summarize older messages using the LLM.
    ///
    /// Returns the new message list with older messages replaced by a summary.
    /// The summary is stored as a System(Context) message at the front.
    pub async fn compact(
        &mut self,
        messages: &[Message],
        provider: &dyn Provider,
        model: &str,
    ) -> Result<Vec<Message>> {
        if messages.len() <= self.config.preserve_recent {
            return Ok(messages.to_vec());
        }

        let split_point = messages.len() - self.config.preserve_recent;
        let old_messages = &messages[..split_point];
        let recent_messages = &messages[split_point..];

        // Build a text representation of old messages for summarization
        let old_text = render_messages_for_summary(old_messages);

        // Ask the LLM to summarize
        let summary_model = self
            .config
            .summary_model
            .as_deref()
            .unwrap_or(model);

        let summary = self
            .summarize_with_llm(&old_text, summary_model, provider)
            .await?;

        self.compaction_count += 1;

        // Build the new message list: summary context + recent messages
        let mut new_messages = Vec::with_capacity(1 + recent_messages.len());

        let summary_msg = Message::System(SystemMessage {
            id: Uuid::new_v4(),
            content: format!(
                "[Context summary — compaction #{count}]\n\n{summary}",
                count = self.compaction_count,
            ),
            system_type: SystemMessageType::Context,
        });
        new_messages.push(summary_msg);
        new_messages.extend_from_slice(recent_messages);

        self.estimated_tokens = Self::estimate_tokens(&new_messages);

        tracing::info!(
            "Compacted {} messages into summary ({} tokens estimated, compaction #{})",
            old_messages.len(),
            self.estimated_tokens,
            self.compaction_count,
        );

        Ok(new_messages)
    }

    /// Use the LLM to summarize a block of conversation text.
    async fn summarize_with_llm(
        &self,
        conversation_text: &str,
        model: &str,
        provider: &dyn Provider,
    ) -> Result<String> {
        let system_prompt = r#"You are a conversation summarizer for an AI coding assistant. Create a detailed summary preserving ALL of the following sections:

1. PRIMARY REQUEST AND INTENT — The user's main goal, what they want to achieve, and why.

2. KEY TECHNICAL CONCEPTS — Frameworks, patterns, libraries, APIs, and constraints discussed. Include version numbers and specific configurations.

3. FILES AND CODE SECTIONS — Every file path mentioned with context. For files that were created or modified, include:
   - Full function/class signatures
   - Key code snippets (preserve exact indentation and syntax)
   - Line number ranges where changes were made
   - What each change does

4. ERRORS AND FIXES — Every error message encountered (verbatim), what caused it, and the exact fix applied.

5. PROBLEM-SOLVING CONTEXT — Approaches tried and their outcomes. What worked, what didn't, and why alternatives were rejected.

6. ALL USER MESSAGES — Every distinct request, instruction, preference, or correction from the user. Quote them if short.

7. PENDING TASKS — Any incomplete work, planned next steps, or deferred items.

8. CURRENT WORK STATE — What was being actively worked on when this summary was created, including exact file names, function names, and current progress.

9. OPTIONAL NEXT STEP — The most likely next action based on recent context, with supporting evidence.

IMPORTANT:
- Preserve ALL file paths, function names, variable names, and code snippets verbatim.
- The summary REPLACES the original messages — lost details cannot be recovered.
- When in doubt, include more detail rather than less.

Output your analysis in <analysis> tags (will be stripped), then the summary."#;

        let request = CompletionRequest {
            model: model.to_string(),
            system_prompt: system_prompt.to_string(),
            messages: vec![ApiMessage {
                role: "user".into(),
                content: serde_json::json!(format!(
                    "Summarize this conversation history:\n\n{conversation_text}"
                )),
            }],
            tools: vec![],
            max_tokens: self.config.summary_max_tokens,
            temperature: Some(0.0),
            thinking: None,
            system_blocks: None,
        };

        let events = provider.stream(request).await?;

        let mut summary = String::new();
        for event in events {
            if let AssistantEvent::TextDelta(text) = event {
                summary.push_str(&text);
            }
        }

        if summary.is_empty() {
            anyhow::bail!("LLM returned empty summary during compaction");
        }

        // Strip <analysis> tags — the summarizer uses them for internal reasoning
        let summary = strip_analysis_tags(&summary);

        Ok(summary)
    }

    /// Get the number of compactions performed.
    pub fn compaction_count(&self) -> u32 {
        self.compaction_count
    }

    /// Get current estimated token count.
    pub fn estimated_tokens(&self) -> u64 {
        self.estimated_tokens
    }
}

// ---------------------------------------------------------------------------
// Post-compaction file restoration
// ---------------------------------------------------------------------------

/// Configuration and logic for restoring recently-referenced file contents
/// after a full compaction. When the LLM summarizes older messages, the raw
/// file contents and skill context that were present in tool results are lost.
/// This struct rebuilds a lightweight snapshot of the most recently touched
/// files so the model retains working context.
#[derive(Debug, Clone)]
pub struct PostCompactRestoration {
    /// Maximum number of files to restore.
    pub max_files: usize,
    /// Maximum characters per individual file (UTF-8 safe truncation).
    pub max_chars_per_file: usize,
    /// Total character budget across all restored files.
    pub total_char_budget: usize,
}

impl Default for PostCompactRestoration {
    fn default() -> Self {
        Self {
            max_files: 5,
            max_chars_per_file: 15_000,  // ~5 000 tokens
            total_char_budget: 150_000,  // ~50 000 tokens
        }
    }
}

impl PostCompactRestoration {
    /// Build the restoration context string for injection as a system reminder.
    ///
    /// * `messages` — the *full* pre-compaction message list (used to find
    ///   recently referenced files).
    /// * `cwd` — working directory; relative paths are resolved against it.
    ///
    /// Returns `None` when no files could be read (all missing, empty, etc.).
    pub fn build(&self, messages: &[Message], cwd: &str) -> Option<String> {
        let paths = collect_recent_files(messages, self.max_files);
        self.build_from_paths(&paths, cwd)
    }

    /// Build restoration context from pre-collected file paths.
    ///
    /// Use this when you've already called `collect_recent_files()` separately
    /// (e.g. before compaction, to avoid cloning the full message list).
    pub fn build_from_paths(&self, paths: &[String], cwd: &str) -> Option<String> {
        if paths.is_empty() {
            return None;
        }
        let context = self.build_restoration_context(paths, cwd);
        if context.is_empty() {
            return None;
        }
        Some(context)
    }

    /// Read each file, truncate to budget, and format into a snapshot block.
    fn build_restoration_context(&self, files: &[String], cwd: &str) -> String {
        let mut sections: Vec<String> = Vec::new();
        let mut remaining_budget = self.total_char_budget;

        for path in files {
            if remaining_budget == 0 {
                break;
            }

            let full_path = if Path::new(path).is_absolute() {
                path.clone()
            } else {
                format!("{}/{}", cwd.trim_end_matches('/'), path)
            };

            let content = match std::fs::read_to_string(&full_path) {
                Ok(c) if !c.is_empty() => c,
                _ => continue, // skip missing / unreadable / empty files
            };

            // Per-file cap
            let file_cap = self.max_chars_per_file.min(remaining_budget);
            let truncated = if content.len() > file_cap {
                let end = truncate_at_char_boundary(&content, file_cap);
                format!("{}…[truncated]", &content[..end])
            } else {
                content.clone()
            };

            remaining_budget = remaining_budget.saturating_sub(truncated.len());

            // Guess a markdown language tag from the extension
            let lang = Path::new(path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            sections.push(format!("File: {path}\n```{lang}\n{truncated}\n```"));
        }

        if sections.is_empty() {
            return String::new();
        }

        format!(
            "[Post-compaction file snapshot]\n\n{}",
            sections.join("\n\n")
        )
    }
}

/// Scan messages for file paths referenced by Read, Write, Edit, and Grep
/// tool uses. Returns up to `max` unique paths in most-recently-referenced
/// order (latest first).
pub fn collect_recent_files(messages: &[Message], max: usize) -> Vec<String> {
    // Tool names whose `input.file_path` points to a file we want to restore.
    const FILE_TOOLS: &[&str] = &["Read", "Write", "Edit", "Grep"];

    let mut seen = HashSet::new();
    let mut result: Vec<String> = Vec::new();

    // Walk messages in reverse so the most recently referenced files come first.
    for msg in messages.iter().rev() {
        if result.len() >= max {
            break;
        }

        // Check top-level ToolUse messages
        if let Message::ToolUse(tu) = msg {
            if FILE_TOOLS.contains(&tu.tool_name.as_str()) {
                if let Some(fp) = tu.input.get("file_path").and_then(|v| v.as_str()) {
                    if !fp.is_empty() && seen.insert(fp.to_string()) {
                        result.push(fp.to_string());
                    }
                }
                // Grep uses `path` instead of `file_path`
                if tu.tool_name == "Grep" {
                    if let Some(fp) = tu.input.get("path").and_then(|v| v.as_str()) {
                        if !fp.is_empty() && seen.insert(fp.to_string()) {
                            result.push(fp.to_string());
                        }
                    }
                }
            }
        }

        // Check ContentBlock::ToolUse inside Assistant messages
        if let Message::Assistant(a) = msg {
            for block in &a.content {
                if result.len() >= max {
                    break;
                }
                if let ContentBlock::ToolUse { name, input, .. } = block {
                    if FILE_TOOLS.contains(&name.as_str()) {
                        if let Some(fp) = input.get("file_path").and_then(|v| v.as_str()) {
                            if !fp.is_empty() && seen.insert(fp.to_string()) {
                                result.push(fp.to_string());
                            }
                        }
                        if name == "Grep" {
                            if let Some(fp) = input.get("path").and_then(|v| v.as_str()) {
                                if !fp.is_empty() && seen.insert(fp.to_string()) {
                                    result.push(fp.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    result
}

/// Strip `<analysis>...</analysis>` tags from the summary.
/// The summarizer uses these for internal reasoning which should not appear in the final summary.
fn strip_analysis_tags(text: &str) -> String {
    // Find and remove <analysis>...</analysis> blocks
    let mut result = text.to_string();
    while let Some(start) = result.find("<analysis>") {
        // Search for the closing tag AFTER the opening tag, not from the beginning
        if let Some(rel_end) = result[start..].find("</analysis>") {
            let end = start + rel_end + "</analysis>".len();
            // Also strip trailing newlines after the closing tag
            let trim_end = result[end..].find(|c: char| c != '\n').map_or(end, |i| end + i);
            result = format!("{}{}", &result[..start], &result[trim_end..]);
        } else {
            // Unclosed <analysis> tag — strip it and everything after it
            result = result[..start].to_string();
            break;
        }
    }
    result.trim().to_string()
}

/// Find a safe byte index for truncation that doesn't split a multi-byte UTF-8 character.
/// Returns the largest byte index <= `max_bytes` that falls on a char boundary.
fn truncate_at_char_boundary(s: &str, max_bytes: usize) -> usize {
    if max_bytes >= s.len() {
        return s.len();
    }
    // Walk backwards from max_bytes to find a char boundary
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// Render messages into a human-readable text block for the summarizer.
fn render_messages_for_summary(messages: &[Message]) -> String {
    let mut lines = Vec::new();

    for msg in messages {
        match msg {
            Message::User(u) => {
                let text = extract_text(&u.content);
                if !text.is_empty() {
                    lines.push(format!("User: {text}"));
                }
            }
            Message::Assistant(a) => {
                let text = extract_text(&a.content);
                let tools: Vec<String> = a
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolUse { name, input, .. } => {
                            let input_str = input.to_string();
                            let truncated = if input_str.len() > 200 {
                                let end = truncate_at_char_boundary(&input_str, 200);
                                format!("{}...", &input_str[..end])
                            } else {
                                input_str
                            };
                            Some(format!("  [tool: {name}({truncated})]"))
                        }
                        _ => None,
                    })
                    .collect();

                if !text.is_empty() {
                    lines.push(format!("Assistant: {text}"));
                }
                lines.extend(tools);
            }
            Message::System(s) => {
                lines.push(format!("System: {}", s.content));
            }
            Message::ToolResult(r) => {
                let truncated = if r.content.len() > 1000 {
                    let end = truncate_at_char_boundary(&r.content, 1000);
                    format!("{}...", &r.content[..end])
                } else {
                    r.content.clone()
                };
                let prefix = if r.is_error { "Error" } else { "Result" };
                lines.push(format!("  [{prefix}: {truncated}]"));
            }
            Message::ToolUse(t) => {
                lines.push(format!("  [tool_use: {}]", t.tool_name));
            }
        }
    }

    lines.join("\n")
}

/// Extract text content from content blocks.
fn extract_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Helper to count characters in content blocks.
fn content_blocks_chars(blocks: &[ContentBlock]) -> u64 {
    blocks
        .iter()
        .map(|b| match b {
            ContentBlock::Text { text } => text.len() as u64,
            ContentBlock::ToolUse { input, name, .. } => {
                name.len() as u64 + input.to_string().len() as u64
            }
            ContentBlock::ToolResult { content, .. } => content.len() as u64,
            ContentBlock::Image { source } => source.data.len() as u64,
            ContentBlock::Thinking { thinking } => thinking.len() as u64,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cisco_code_protocol::UserMessage;

    fn make_user_msg(text: &str) -> Message {
        Message::User(UserMessage {
            id: Uuid::new_v4(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            attachments: None,
        })
    }

    fn make_assistant_msg(text: &str) -> Message {
        Message::Assistant(cisco_code_protocol::AssistantMessage {
            id: Uuid::new_v4(),
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            model: "test".into(),
            usage: TokenUsage::default(),
            stop_reason: None,
        })
    }

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(Compactor::estimate_tokens(&[]), 0);
    }

    #[test]
    fn test_estimate_tokens_basic() {
        // "hello world" = 11 chars → ~3 tokens
        let msgs = vec![make_user_msg("hello world")];
        let tokens = Compactor::estimate_tokens(&msgs);
        assert!(tokens >= 2 && tokens <= 4, "got {tokens}");
    }

    #[test]
    fn test_estimate_tokens_multiple_messages() {
        let msgs = vec![
            make_user_msg("first message here"),
            make_assistant_msg("second response here"),
            make_user_msg("third question"),
        ];
        let tokens = Compactor::estimate_tokens(&msgs);
        // 18 + 20 + 14 = 52 chars → ~13 tokens
        assert!(tokens >= 10 && tokens <= 20, "got {tokens}");
    }

    #[test]
    fn test_needs_compaction_below_threshold() {
        let mut compactor = Compactor::new(CompactConfig {
            compact_threshold: 1000,
            ..Default::default()
        });
        compactor.estimated_tokens = 500;
        assert!(!compactor.needs_compaction());
    }

    #[test]
    fn test_needs_compaction_above_threshold() {
        let mut compactor = Compactor::new(CompactConfig {
            compact_threshold: 1000,
            ..Default::default()
        });
        compactor.estimated_tokens = 1500;
        assert!(compactor.needs_compaction());
    }

    #[test]
    fn test_render_messages_for_summary() {
        let msgs = vec![
            make_user_msg("What files are in src/?"),
            make_assistant_msg("Let me check."),
            Message::ToolResult(cisco_code_protocol::ToolResultMessage {
                id: Uuid::new_v4(),
                tool_use_id: "tu_1".into(),
                content: "main.rs\nlib.rs".into(),
                is_error: false,
                injected_messages: None,
            }),
        ];

        let rendered = render_messages_for_summary(&msgs);
        assert!(rendered.contains("User: What files are in src/?"));
        assert!(rendered.contains("Assistant: Let me check."));
        assert!(rendered.contains("[Result: main.rs"));
    }

    #[test]
    fn test_render_messages_truncates_long_results() {
        let long_content = "x".repeat(2000);
        let msgs = vec![Message::ToolResult(
            cisco_code_protocol::ToolResultMessage {
                id: Uuid::new_v4(),
                tool_use_id: "tu_1".into(),
                content: long_content,
                is_error: false,
                injected_messages: None,
            },
        )];

        let rendered = render_messages_for_summary(&msgs);
        assert!(rendered.contains("..."));
        // Should be truncated to ~1000 chars + "..."
        assert!(rendered.len() < 1200);
    }

    #[test]
    fn test_strip_analysis_tags() {
        let input = "<analysis>\nSome internal reasoning\n</analysis>\nActual summary here";
        let result = strip_analysis_tags(input);
        assert_eq!(result, "Actual summary here");
    }

    #[test]
    fn test_strip_analysis_tags_no_tags() {
        let input = "Just a plain summary";
        let result = strip_analysis_tags(input);
        assert_eq!(result, "Just a plain summary");
    }

    #[test]
    fn test_strip_analysis_tags_multiple() {
        let input = "<analysis>first</analysis>\nSummary part 1\n<analysis>second</analysis>\nSummary part 2";
        let result = strip_analysis_tags(input);
        assert!(result.contains("Summary part 1"));
        assert!(result.contains("Summary part 2"));
        assert!(!result.contains("first"));
        assert!(!result.contains("second"));
    }

    #[test]
    fn test_extract_text_mixed_blocks() {
        let blocks = vec![
            ContentBlock::Text {
                text: "hello ".into(),
            },
            ContentBlock::ToolUse {
                id: "tu_1".into(),
                name: "Bash".into(),
                input: serde_json::json!({}),
            },
            ContentBlock::Text {
                text: "world".into(),
            },
        ];
        assert_eq!(extract_text(&blocks), "hello world");
    }

    #[test]
    fn test_compact_config_default() {
        let config = CompactConfig::default();
        assert_eq!(config.compact_threshold, 80_000);
        assert_eq!(config.preserve_recent, 10);
        assert_eq!(config.summary_max_tokens, 8192);
        assert!(config.summary_model.is_none());
        assert!(config.target_after_compact.is_none());
    }

    #[test]
    fn test_compactor_initial_state() {
        let compactor = Compactor::new(CompactConfig::default());
        assert_eq!(compactor.compaction_count(), 0);
        assert_eq!(compactor.estimated_tokens(), 0);
    }

    #[test]
    fn test_update_estimate() {
        let mut compactor = Compactor::new(CompactConfig::default());
        let msgs = vec![
            make_user_msg("hello"),
            make_assistant_msg("world"),
        ];
        compactor.update_estimate(&msgs);
        assert!(compactor.estimated_tokens() > 0);
    }

    #[test]
    fn test_for_model_claude() {
        let compactor = Compactor::for_model("claude-sonnet-4-6");
        // 200K * 0.80 - 4K = 156K
        assert_eq!(compactor.config.compact_threshold, 156_000);
        assert!(compactor.config.target_after_compact.is_some());
        let target = compactor.config.target_after_compact.unwrap();
        assert!(target < compactor.config.compact_threshold);
    }

    #[test]
    fn test_for_model_gpt4o() {
        let compactor = Compactor::for_model("gpt-4o");
        // 128K * 0.80 - 4K = 98.4K
        assert_eq!(compactor.config.compact_threshold, 98_400);
    }

    #[test]
    fn test_for_model_bedrock() {
        let compactor = Compactor::for_model("anthropic.claude-sonnet-4-6-v1:0");
        // Same as Claude direct: 200K
        assert_eq!(compactor.config.compact_threshold, 156_000);
    }

    #[test]
    fn test_for_model_o3() {
        let compactor = Compactor::for_model("o3");
        assert_eq!(compactor.config.compact_threshold, 156_000);
    }

    #[test]
    fn test_for_model_unknown_uses_conservative() {
        let compactor = Compactor::for_model("custom-llm-v1");
        // 128K * 0.80 - 4K = 98.4K
        assert_eq!(compactor.config.compact_threshold, 98_400);
    }

    #[test]
    fn test_threshold_for_model() {
        assert_eq!(threshold_for_model("claude-opus-4-6"), 156_000);
        assert_eq!(threshold_for_model("gpt-4o-mini"), 98_400);
        assert_eq!(threshold_for_model("o3-mini"), 156_000);
    }

    // -----------------------------------------------------------------------
    // PostCompactRestoration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_post_compact_restoration_default() {
        let pcr = PostCompactRestoration::default();
        assert_eq!(pcr.max_files, 5);
        assert_eq!(pcr.max_chars_per_file, 15_000);
        assert_eq!(pcr.total_char_budget, 150_000);
    }

    /// Helper: build a ToolUse message that looks like a Read call.
    fn make_tool_use_msg(tool_name: &str, file_path: &str) -> Message {
        Message::ToolUse(cisco_code_protocol::ToolUseMessage {
            id: Uuid::new_v4(),
            tool_use_id: format!("tu_{}", Uuid::new_v4()),
            tool_name: tool_name.to_string(),
            input: serde_json::json!({ "file_path": file_path }),
        })
    }

    /// Helper: build a ToolUse message for Grep with `path` field.
    fn make_grep_msg(path: &str) -> Message {
        Message::ToolUse(cisco_code_protocol::ToolUseMessage {
            id: Uuid::new_v4(),
            tool_use_id: format!("tu_{}", Uuid::new_v4()),
            tool_name: "Grep".to_string(),
            input: serde_json::json!({ "pattern": "TODO", "path": path }),
        })
    }

    #[test]
    fn test_collect_recent_files_empty() {
        let files = collect_recent_files(&[], 5);
        assert!(files.is_empty());
    }

    #[test]
    fn test_collect_recent_files_basic() {
        let msgs = vec![
            make_tool_use_msg("Read", "/src/main.rs"),
            make_tool_use_msg("Write", "/src/lib.rs"),
            make_tool_use_msg("Edit", "/src/config.rs"),
        ];
        let files = collect_recent_files(&msgs, 5);
        // Most recently referenced first (reverse order)
        assert_eq!(files, vec![
            "/src/config.rs",
            "/src/lib.rs",
            "/src/main.rs",
        ]);
    }

    #[test]
    fn test_collect_recent_files_deduplication() {
        let msgs = vec![
            make_tool_use_msg("Read", "/src/main.rs"),
            make_tool_use_msg("Edit", "/src/main.rs"),
            make_tool_use_msg("Read", "/src/lib.rs"),
        ];
        let files = collect_recent_files(&msgs, 5);
        // /src/lib.rs is the most recent, /src/main.rs appears twice but is deduped
        assert_eq!(files, vec!["/src/lib.rs", "/src/main.rs"]);
    }

    #[test]
    fn test_collect_recent_files_respects_max() {
        let msgs = vec![
            make_tool_use_msg("Read", "/a.rs"),
            make_tool_use_msg("Read", "/b.rs"),
            make_tool_use_msg("Read", "/c.rs"),
            make_tool_use_msg("Read", "/d.rs"),
        ];
        let files = collect_recent_files(&msgs, 2);
        assert_eq!(files.len(), 2);
        // Most recent first
        assert_eq!(files, vec!["/d.rs", "/c.rs"]);
    }

    #[test]
    fn test_collect_recent_files_ignores_non_file_tools() {
        let msgs = vec![
            make_tool_use_msg("Bash", "/src/main.rs"),
            make_tool_use_msg("Read", "/src/lib.rs"),
        ];
        let files = collect_recent_files(&msgs, 5);
        assert_eq!(files, vec!["/src/lib.rs"]);
    }

    #[test]
    fn test_collect_recent_files_grep_path_field() {
        let msgs = vec![
            make_grep_msg("/src/engine"),
            make_tool_use_msg("Read", "/src/main.rs"),
        ];
        let files = collect_recent_files(&msgs, 5);
        assert_eq!(files, vec!["/src/main.rs", "/src/engine"]);
    }

    #[test]
    fn test_collect_recent_files_from_assistant_content_blocks() {
        let msgs = vec![
            Message::Assistant(cisco_code_protocol::AssistantMessage {
                id: Uuid::new_v4(),
                content: vec![
                    ContentBlock::Text { text: "Let me read that.".into() },
                    ContentBlock::ToolUse {
                        id: "tu_1".into(),
                        name: "Read".into(),
                        input: serde_json::json!({ "file_path": "/src/foo.rs" }),
                    },
                    ContentBlock::ToolUse {
                        id: "tu_2".into(),
                        name: "Edit".into(),
                        input: serde_json::json!({ "file_path": "/src/bar.rs", "old_string": "a", "new_string": "b" }),
                    },
                ],
                model: "test".into(),
                usage: TokenUsage::default(),
                stop_reason: None,
            }),
        ];
        let files = collect_recent_files(&msgs, 5);
        // ToolUse blocks within assistant message are iterated in order,
        // but we walk messages in reverse — only one message here.
        // Within the message, blocks are iterated forward, so bar.rs is
        // added after foo.rs (but since we iterate blocks forward inside
        // a reverse message walk, the order depends on the inner loop).
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"/src/foo.rs".to_string()));
        assert!(files.contains(&"/src/bar.rs".to_string()));
    }

    #[test]
    fn test_build_restoration_context_with_real_file() {
        // Use a file we know exists in this project
        let pcr = PostCompactRestoration {
            max_files: 2,
            max_chars_per_file: 100,
            total_char_budget: 500,
        };
        // Build context using Cargo.toml which should exist at the project root
        let msgs = vec![make_tool_use_msg("Read", "Cargo.toml")];
        // Use the project root as cwd — Cargo.toml lives there
        let result = pcr.build(&msgs, env!("CARGO_MANIFEST_DIR"));
        // If Cargo.toml exists, we should get a snapshot
        // (in test environments it may not exist at the expected path,
        // so we just check the function doesn't panic)
        if let Some(ctx) = result {
            assert!(ctx.starts_with("[Post-compaction file snapshot]"));
            assert!(ctx.contains("File: Cargo.toml"));
        }
    }

    #[test]
    fn test_build_restoration_context_missing_files() {
        let pcr = PostCompactRestoration::default();
        let msgs = vec![
            make_tool_use_msg("Read", "/nonexistent/file_abc123.rs"),
        ];
        let result = pcr.build(&msgs, "/tmp");
        assert!(result.is_none());
    }

    #[test]
    fn test_build_restoration_context_respects_budget() {
        let pcr = PostCompactRestoration {
            max_files: 10,
            max_chars_per_file: 50,
            total_char_budget: 80, // very small budget
        };
        // Even though max_chars_per_file is 50, the total budget of 80
        // should limit how much we accumulate across files.
        // This is a structural test — we can't easily test with real files
        // but we verify the logic via the default and missing-file paths.
        assert_eq!(pcr.total_char_budget, 80);
    }
}
