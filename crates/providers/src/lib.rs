//! cisco-code-providers: LLM provider adapters in pure Rust.
//!
//! Every LLM API is just HTTP + SSE. No Python SDKs needed.
//! Codex and Claw-Code-Parity both prove this works with reqwest + serde_json.
//!
//! Supported providers:
//! - Anthropic (Claude) — Messages API with SSE streaming
//! - OpenAI (GPT) — Chat Completions API with SSE streaming
//! - Google (Gemini) — GenerateContent API
//! - AWS Bedrock — InvokeModel API with SigV4 auth
//! - Cisco AI Cloud — Internal model hosting
//! - OpenAI-compatible — Any endpoint following the OpenAI format
//!   (DeepSeek, Groq, Ollama, vLLM, SGLang, Mistral, xAI, etc.)

pub mod registry;
pub mod anthropic;
pub mod openai;
pub mod openai_compat;
pub mod routing;

pub use registry::*;
pub use routing::*;

/// Model capability tiers — the right model for each task.
///
/// Design insight from Astro-Assistant: 5-tier routing saves 80%+ on LLM costs
/// while maintaining quality where it matters. Title generation doesn't need Opus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    /// Titles, classification, simple queries (~$0.001/1K tokens)
    Small,
    /// Compaction, code review, summarization (~$0.003/1K tokens)
    Medium,
    /// Main agent, execution, coding tasks (~$0.015/1K tokens)
    Large,
    /// Peer review, second opinion from different vendor
    Teammate,
    /// Complex planning, research, architecture
    Frontier,
}

/// Agent roles that map to model tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    MainAgent,
    Planner,
    Executor,
    Reviewer,
    Compaction,
    Classifier,
    Title,
    Guardian,
}
