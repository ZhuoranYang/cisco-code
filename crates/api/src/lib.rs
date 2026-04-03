//! cisco-code-api: HTTP client with SSE streaming and transport fallback.
//!
//! Design insight from Codex: Use transport fallback (WebSocket → SSE → REST)
//! for maximum compatibility across providers and network conditions.
//!
//! Design insight from Claw-Code-Parity: The ApiClient trait is generic,
//! enabling mock clients for testing and swappable backends at runtime.

pub mod client;
pub mod sse;

pub use client::*;
