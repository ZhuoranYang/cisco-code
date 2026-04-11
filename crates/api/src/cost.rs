//! Cost tracking for LLM API usage.
//!
//! Tracks token usage, API duration, and estimated costs per session.
//! Matches Claude Code's cost-tracker.ts with per-model breakdowns.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Per-model usage breakdown.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_usd: f64,
}

/// Session cost state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostState {
    pub total_cost_usd: f64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_api_calls: u64,
    pub total_api_duration_ms: u64,
    pub total_tool_calls: u64,
    pub lines_added: u64,
    pub lines_removed: u64,
    pub model_usage: HashMap<String, ModelUsage>,
}

/// Pricing per million tokens (input, output) in USD.
fn pricing_per_million(model: &str) -> (f64, f64) {
    // Match on model family prefixes
    if model.contains("opus") {
        (15.0, 75.0) // Claude Opus
    } else if model.contains("sonnet") {
        (3.0, 15.0) // Claude Sonnet
    } else if model.contains("haiku") {
        (0.25, 1.25) // Claude Haiku
    } else if model.contains("gpt-4o-mini") {
        (0.15, 0.60) // GPT-4o Mini
    } else if model.contains("gpt-4o") {
        (2.50, 10.0) // GPT-4o
    } else if model.starts_with("o3-mini") {
        (1.10, 4.40) // o3-mini
    } else if model.starts_with("o3") {
        (10.0, 40.0) // o3
    } else if model.starts_with("o1") {
        (15.0, 60.0) // o1
    } else {
        (3.0, 15.0) // Default to Sonnet-like pricing
    }
}

/// Calculate cost in USD for token usage.
pub fn calculate_cost(model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let (input_price, output_price) = pricing_per_million(model);
    let input_cost = input_tokens as f64 * input_price / 1_000_000.0;
    let output_cost = output_tokens as f64 * output_price / 1_000_000.0;
    input_cost + output_cost
}

/// Cost tracker for a session.
pub struct CostTracker {
    state: CostState,
    session_start: Instant,
}

impl CostTracker {
    pub fn new() -> Self {
        Self {
            state: CostState::default(),
            session_start: Instant::now(),
        }
    }

    /// Record usage from an API call.
    pub fn record_usage(
        &mut self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        api_duration: Duration,
    ) {
        let cost = calculate_cost(model, input_tokens, output_tokens);

        self.state.total_cost_usd += cost;
        self.state.total_input_tokens += input_tokens;
        self.state.total_output_tokens += output_tokens;
        self.state.total_api_calls += 1;
        self.state.total_api_duration_ms += api_duration.as_millis() as u64;

        let usage = self
            .state
            .model_usage
            .entry(model.to_string())
            .or_default();
        usage.input_tokens += input_tokens;
        usage.output_tokens += output_tokens;
        usage.cost_usd += cost;
    }

    /// Record cache tokens.
    pub fn record_cache(
        &mut self,
        model: &str,
        cache_read: u64,
        cache_write: u64,
    ) {
        let usage = self
            .state
            .model_usage
            .entry(model.to_string())
            .or_default();
        usage.cache_read_tokens += cache_read;
        usage.cache_write_tokens += cache_write;
    }

    /// Record tool usage.
    pub fn record_tool_call(&mut self) {
        self.state.total_tool_calls += 1;
    }

    /// Record lines changed.
    pub fn record_lines_changed(&mut self, added: u64, removed: u64) {
        self.state.lines_added += added;
        self.state.lines_removed += removed;
    }

    /// Get current cost state.
    pub fn state(&self) -> &CostState {
        &self.state
    }

    /// Total session elapsed time.
    pub fn elapsed(&self) -> Duration {
        self.session_start.elapsed()
    }

    /// Format cost as display string.
    pub fn format_cost(&self) -> String {
        let cost = self.state.total_cost_usd;
        if cost < 0.01 {
            format!("${:.4}", cost)
        } else {
            format!("${:.2}", cost)
        }
    }

    /// Format a summary of usage.
    pub fn format_summary(&self) -> String {
        let mut summary = String::new();
        summary.push_str(&format!("Total cost: {}\n", self.format_cost()));
        summary.push_str(&format!(
            "Tokens: {} in / {} out\n",
            format_tokens(self.state.total_input_tokens),
            format_tokens(self.state.total_output_tokens),
        ));
        summary.push_str(&format!("API calls: {}\n", self.state.total_api_calls));
        summary.push_str(&format!("Tool calls: {}\n", self.state.total_tool_calls));

        if !self.state.model_usage.is_empty() {
            summary.push_str("\nPer-model breakdown:\n");
            let mut models: Vec<_> = self.state.model_usage.iter().collect();
            models.sort_by(|a, b| b.1.cost_usd.partial_cmp(&a.1.cost_usd).unwrap());
            for (model, usage) in models {
                summary.push_str(&format!(
                    "  {}: {} in / {} out (${:.4})\n",
                    model,
                    format_tokens(usage.input_tokens),
                    format_tokens(usage.output_tokens),
                    usage.cost_usd,
                ));
            }
        }

        summary
    }

    /// Serialize state to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.state).unwrap_or_else(|_| "{}".into())
    }

    /// Restore from a previous state.
    pub fn restore(&mut self, state: CostState) {
        self.state = state;
    }
}

impl Default for CostTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Format token count for display (K/M notation).
fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_cost_sonnet() {
        let cost = calculate_cost("claude-sonnet-4-6", 1_000_000, 1_000_000);
        assert!((cost - 18.0).abs() < 0.01); // $3 input + $15 output
    }

    #[test]
    fn test_calculate_cost_opus() {
        let cost = calculate_cost("claude-opus-4-6", 1_000_000, 1_000_000);
        assert!((cost - 90.0).abs() < 0.01); // $15 input + $75 output
    }

    #[test]
    fn test_calculate_cost_haiku() {
        let cost = calculate_cost("claude-haiku-4-5", 1_000_000, 1_000_000);
        assert!((cost - 1.5).abs() < 0.01); // $0.25 input + $1.25 output
    }

    #[test]
    fn test_calculate_cost_gpt4o() {
        let cost = calculate_cost("gpt-4o", 1_000_000, 1_000_000);
        assert!((cost - 12.5).abs() < 0.01); // $2.50 input + $10 output
    }

    #[test]
    fn test_calculate_cost_gpt4o_mini() {
        let cost = calculate_cost("gpt-4o-mini", 1_000_000, 1_000_000);
        assert!((cost - 0.75).abs() < 0.01); // $0.15 + $0.60
    }

    #[test]
    fn test_cost_tracker_basic() {
        let mut tracker = CostTracker::new();
        tracker.record_usage("claude-sonnet-4-6", 1000, 500, Duration::from_millis(200));

        assert_eq!(tracker.state().total_api_calls, 1);
        assert_eq!(tracker.state().total_input_tokens, 1000);
        assert_eq!(tracker.state().total_output_tokens, 500);
        assert!(tracker.state().total_cost_usd > 0.0);
    }

    #[test]
    fn test_cost_tracker_multiple_models() {
        let mut tracker = CostTracker::new();
        tracker.record_usage("claude-sonnet-4-6", 1000, 500, Duration::from_millis(100));
        tracker.record_usage("gpt-4o", 2000, 1000, Duration::from_millis(150));

        assert_eq!(tracker.state().total_api_calls, 2);
        assert_eq!(tracker.state().total_input_tokens, 3000);
        assert_eq!(tracker.state().model_usage.len(), 2);
    }

    #[test]
    fn test_cost_tracker_tool_calls() {
        let mut tracker = CostTracker::new();
        tracker.record_tool_call();
        tracker.record_tool_call();
        assert_eq!(tracker.state().total_tool_calls, 2);
    }

    #[test]
    fn test_cost_tracker_lines_changed() {
        let mut tracker = CostTracker::new();
        tracker.record_lines_changed(100, 50);
        assert_eq!(tracker.state().lines_added, 100);
        assert_eq!(tracker.state().lines_removed, 50);
    }

    #[test]
    fn test_format_cost() {
        let mut tracker = CostTracker::new();
        tracker.record_usage("claude-sonnet-4-6", 10000, 5000, Duration::from_millis(100));
        let cost_str = tracker.format_cost();
        assert!(cost_str.starts_with('$'));
    }

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(500), "500");
        assert_eq!(format_tokens(1500), "1.5K");
        assert_eq!(format_tokens(1_500_000), "1.5M");
    }

    #[test]
    fn test_format_summary() {
        let mut tracker = CostTracker::new();
        tracker.record_usage("claude-sonnet-4-6", 10000, 5000, Duration::from_millis(200));
        tracker.record_tool_call();
        let summary = tracker.format_summary();
        assert!(summary.contains("Total cost:"));
        assert!(summary.contains("Tokens:"));
        assert!(summary.contains("claude-sonnet"));
    }

    #[test]
    fn test_cost_state_serialization() {
        let mut tracker = CostTracker::new();
        tracker.record_usage("claude-sonnet-4-6", 1000, 500, Duration::from_millis(100));
        let json = tracker.to_json();
        let parsed: CostState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.total_api_calls, 1);
    }

    #[test]
    fn test_cost_tracker_restore() {
        let state = CostState {
            total_cost_usd: 1.5,
            total_input_tokens: 10000,
            total_output_tokens: 5000,
            total_api_calls: 3,
            ..Default::default()
        };
        let mut tracker = CostTracker::new();
        tracker.restore(state);
        assert_eq!(tracker.state().total_api_calls, 3);
        assert!((tracker.state().total_cost_usd - 1.5).abs() < 0.001);
    }

    #[test]
    fn test_record_cache() {
        let mut tracker = CostTracker::new();
        tracker.record_usage("claude-sonnet-4-6", 1000, 500, Duration::from_millis(100));
        tracker.record_cache("claude-sonnet-4-6", 5000, 2000);
        let usage = tracker.state().model_usage.get("claude-sonnet-4-6").unwrap();
        assert_eq!(usage.cache_read_tokens, 5000);
        assert_eq!(usage.cache_write_tokens, 2000);
    }

    #[test]
    fn test_default_tracker() {
        let tracker = CostTracker::default();
        assert_eq!(tracker.state().total_api_calls, 0);
    }

    #[test]
    fn test_o3_pricing() {
        let cost = calculate_cost("o3", 1_000_000, 1_000_000);
        assert!((cost - 50.0).abs() < 0.01); // $10 + $40
    }

    #[test]
    fn test_bedrock_model_pricing() {
        // Bedrock models use same base pricing
        let cost = calculate_cost("anthropic.claude-sonnet-4-6-v1:0", 1_000_000, 1_000_000);
        assert!((cost - 18.0).abs() < 0.01); // Same as direct Sonnet
    }
}
