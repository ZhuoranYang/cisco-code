//! cisco-code-api: HTTP client with SSE streaming and transport fallback.
//!
//! Design insight from Codex: Use transport fallback (WebSocket → SSE → REST)
//! for maximum compatibility across providers and network conditions.
//!
//! Design insight from Claw-Code-Parity: The ApiClient trait is generic,
//! enabling mock clients for testing and swappable backends at runtime.

pub mod bedrock;
pub mod client;
pub mod cost;
pub mod oauth;
pub mod openai;
pub mod registry;
pub mod sse;

pub use client::*;
pub use cost::{calculate_cost, CostState, CostTracker, ModelUsage};
pub use registry::{
    builtin_models, resolve_model_provider, ModelInfo, ProviderConfig, ProviderRegistry,
    ProviderType,
};
