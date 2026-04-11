//! cisco-code-telemetry: Observability, tracing, metrics, and audit logging.
//!
//! Provides:
//! - Span-based tracing (interaction → llm_request → tool hierarchy)
//! - Metrics collection (tokens, latency, error rates)
//! - SQLite audit log for enterprise compliance
//! - Export in JSON and CSV formats

pub mod audit;
pub mod export;
pub mod metrics;
pub mod spans;

pub use audit::{AuditEntry, AuditLogger};
pub use export::{export_audit_csv, export_metrics_json, export_spans_json};
pub use metrics::{HistogramSummary, MetricsCollector, SessionMetrics};
pub use spans::{Span, SpanCollector, SpanKind, SpanStatus};
