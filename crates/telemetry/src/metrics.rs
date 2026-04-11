//! Metrics collection: counters, gauges, histograms, and session summaries.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Aggregated metrics for a session.
pub struct MetricsCollector {
    session_id: String,
    counters: HashMap<String, u64>,
    gauges: HashMap<String, f64>,
    histograms: HashMap<String, Vec<f64>>,
    start_time: DateTime<Utc>,
}

impl MetricsCollector {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            counters: HashMap::new(),
            gauges: HashMap::new(),
            histograms: HashMap::new(),
            start_time: Utc::now(),
        }
    }

    /// Increment a counter by delta.
    pub fn increment(&mut self, name: &str, delta: u64) {
        *self.counters.entry(name.to_string()).or_insert(0) += delta;
    }

    /// Set a gauge value.
    pub fn set_gauge(&mut self, name: &str, value: f64) {
        self.gauges.insert(name.to_string(), value);
    }

    /// Record a histogram sample.
    pub fn record_histogram(&mut self, name: &str, value: f64) {
        self.histograms
            .entry(name.to_string())
            .or_default()
            .push(value);
    }

    /// Get a counter value.
    pub fn get_counter(&self, name: &str) -> u64 {
        self.counters.get(name).copied().unwrap_or(0)
    }

    /// Get a gauge value.
    pub fn get_gauge(&self, name: &str) -> Option<f64> {
        self.gauges.get(name).copied()
    }

    /// Get a histogram summary.
    pub fn histogram_summary(&self, name: &str) -> Option<HistogramSummary> {
        let values = self.histograms.get(name)?;
        if values.is_empty() {
            return None;
        }

        let mut sorted = values.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let count = sorted.len();
        let sum: f64 = sorted.iter().sum();
        let min = sorted[0];
        let max = sorted[count - 1];
        let mean = sum / count as f64;

        let p50 = percentile(&sorted, 50.0);
        let p95 = percentile(&sorted, 95.0);

        Some(HistogramSummary {
            min,
            max,
            mean,
            p50,
            p95,
            count,
        })
    }

    /// Convert to session metrics.
    ///
    /// Uses well-known counter names:
    /// - `tokens` for total token count
    /// - `turns` for turn count
    /// - `tool_calls` for tool execution count
    /// - `errors` for error count
    /// - histogram `latency_ms` for average latency
    pub fn to_session_metrics(&self, model: &str) -> SessionMetrics {
        let elapsed = Utc::now()
            .signed_duration_since(self.start_time)
            .num_milliseconds() as f64
            / 1000.0;

        let avg_latency_ms = self
            .histogram_summary("latency_ms")
            .map(|h| h.mean)
            .unwrap_or(0.0);

        SessionMetrics {
            session_id: self.session_id.clone(),
            model_name: model.to_string(),
            total_tokens: self.get_counter("tokens"),
            total_turns: self.get_counter("turns"),
            total_tool_calls: self.get_counter("tool_calls"),
            total_errors: self.get_counter("errors"),
            elapsed_secs: elapsed,
            avg_latency_ms,
        }
    }

    /// Elapsed time since creation.
    pub fn elapsed_secs(&self) -> f64 {
        Utc::now()
            .signed_duration_since(self.start_time)
            .num_milliseconds() as f64
            / 1000.0
    }
}

/// Summary statistics for a histogram.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramSummary {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub count: usize,
}

/// Session-level metrics summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetrics {
    pub session_id: String,
    pub model_name: String,
    pub total_tokens: u64,
    pub total_turns: u64,
    pub total_tool_calls: u64,
    pub total_errors: u64,
    pub elapsed_secs: f64,
    pub avg_latency_ms: f64,
}

/// Index-based percentile calculation on sorted values.
///
/// Uses linear interpolation between adjacent ranks for sub-index percentiles.
fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let rank = (pct / 100.0) * (sorted.len() - 1) as f64;
    let lower = rank.floor() as usize;
    let upper = rank.ceil() as usize;
    if lower == upper {
        sorted[lower]
    } else {
        let frac = rank - lower as f64;
        sorted[lower] * (1.0 - frac) + sorted[upper] * frac
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counter_increment() {
        let mut m = MetricsCollector::new("sess-1");
        m.increment("requests", 1);
        m.increment("requests", 2);
        assert_eq!(m.get_counter("requests"), 3);
        assert_eq!(m.get_counter("nonexistent"), 0);
    }

    #[test]
    fn test_counter_independent_names() {
        let mut m = MetricsCollector::new("s1");
        m.increment("a", 10);
        m.increment("b", 20);
        assert_eq!(m.get_counter("a"), 10);
        assert_eq!(m.get_counter("b"), 20);
    }

    #[test]
    fn test_gauge() {
        let mut m = MetricsCollector::new("sess-1");
        assert!(m.get_gauge("temp").is_none());
        m.set_gauge("temp", 98.6);
        assert_eq!(m.get_gauge("temp"), Some(98.6));
        m.set_gauge("temp", 99.1);
        assert_eq!(m.get_gauge("temp"), Some(99.1));
    }

    #[test]
    fn test_histogram_basic() {
        let mut m = MetricsCollector::new("s");
        for v in [10.0, 20.0, 30.0, 40.0, 50.0] {
            m.record_histogram("latency", v);
        }
        let summary = m.histogram_summary("latency").unwrap();
        assert_eq!(summary.min, 10.0);
        assert_eq!(summary.max, 50.0);
        assert_eq!(summary.mean, 30.0);
        assert_eq!(summary.count, 5);
    }

    #[test]
    fn test_histogram_percentiles() {
        let mut m = MetricsCollector::new("s");
        // 5 values: 10, 20, 30, 40, 50
        for v in [10.0, 20.0, 30.0, 40.0, 50.0] {
            m.record_histogram("values", v);
        }
        let summary = m.histogram_summary("values").unwrap();
        // p50: rank = 0.50 * 4 = 2.0 -> sorted[2] = 30.0
        assert!((summary.p50 - 30.0).abs() < f64::EPSILON);
        // p95: rank = 0.95 * 4 = 3.8 -> interpolate: 40*0.2 + 50*0.8 = 48.0
        assert!((summary.p95 - 48.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_histogram_single_value() {
        let mut m = MetricsCollector::new("s");
        m.record_histogram("one", 42.0);
        let summary = m.histogram_summary("one").unwrap();
        assert_eq!(summary.min, 42.0);
        assert_eq!(summary.max, 42.0);
        assert_eq!(summary.mean, 42.0);
        assert_eq!(summary.p50, 42.0);
        assert_eq!(summary.p95, 42.0);
        assert_eq!(summary.count, 1);
    }

    #[test]
    fn test_histogram_nonexistent() {
        let m = MetricsCollector::new("s");
        assert!(m.histogram_summary("nope").is_none());
    }

    #[test]
    fn test_histogram_unordered_input() {
        let mut m = MetricsCollector::new("s");
        for v in [50.0, 10.0, 30.0, 20.0, 40.0] {
            m.record_histogram("lat", v);
        }
        let s = m.histogram_summary("lat").unwrap();
        assert_eq!(s.min, 10.0);
        assert_eq!(s.max, 50.0);
    }

    #[test]
    fn test_percentile_two_values() {
        let mut m = MetricsCollector::new("s");
        m.record_histogram("x", 0.0);
        m.record_histogram("x", 100.0);
        let s = m.histogram_summary("x").unwrap();
        assert!((s.p50 - 50.0).abs() < f64::EPSILON);
        assert!((s.p95 - 95.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_session_metrics() {
        let mut m = MetricsCollector::new("sess-1");
        m.increment("tokens", 1500);
        m.increment("turns", 3);
        m.increment("tool_calls", 5);
        m.increment("errors", 1);
        m.record_histogram("latency_ms", 200.0);
        m.record_histogram("latency_ms", 300.0);

        let session = m.to_session_metrics("claude-sonnet-4-6");
        assert_eq!(session.session_id, "sess-1");
        assert_eq!(session.model_name, "claude-sonnet-4-6");
        assert_eq!(session.total_tokens, 1500);
        assert_eq!(session.total_turns, 3);
        assert_eq!(session.total_tool_calls, 5);
        assert_eq!(session.total_errors, 1);
        assert_eq!(session.avg_latency_ms, 250.0);
        assert!(session.elapsed_secs >= 0.0);
    }

    #[test]
    fn test_session_metrics_defaults() {
        let m = MetricsCollector::new("empty");
        let sm = m.to_session_metrics("gpt-4");
        assert_eq!(sm.total_tokens, 0);
        assert_eq!(sm.total_turns, 0);
        assert_eq!(sm.total_tool_calls, 0);
        assert_eq!(sm.total_errors, 0);
        assert_eq!(sm.avg_latency_ms, 0.0);
    }

    #[test]
    fn test_elapsed() {
        let m = MetricsCollector::new("s");
        assert!(m.elapsed_secs() >= 0.0);
    }

    #[test]
    fn test_percentile_fn() {
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0, 5.0], 50.0), 3.0);
        assert_eq!(percentile(&[], 50.0), 0.0);
        assert_eq!(percentile(&[42.0], 99.0), 42.0);
    }

    #[test]
    fn test_histogram_summary_serialization() {
        let summary = HistogramSummary {
            min: 1.0,
            max: 100.0,
            mean: 50.0,
            p50: 50.0,
            p95: 95.0,
            count: 100,
        };
        let json = serde_json::to_string(&summary).unwrap();
        let parsed: HistogramSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.count, 100);
        assert_eq!(parsed.mean, 50.0);
    }
}
