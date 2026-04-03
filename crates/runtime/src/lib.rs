//! cisco-code-runtime: Core agent loop and session management.
//!
//! The runtime implements `ConversationRuntime<P, T>`, the central orchestration
//! engine that drives the agent loop. It is generic over:
//! - `P: Provider` — the LLM backend (Anthropic, OpenAI, etc.)
//! - `T: ToolExecutor` — the tool execution engine
//!
//! Design insight from Claw-Code-Parity: Making the runtime generic enables
//! testability (mock providers), backend swapping, and clean separation of concerns.
//!
//! Design insight from Claude Code: The runtime manages a rich context including
//! message history, file state cache, permission rules, and hook pipelines.

pub mod conversation;
pub mod config;
pub mod session;
pub mod permissions;
pub mod hooks;
pub mod prompt;
pub mod compact;

pub use conversation::*;
pub use config::*;
