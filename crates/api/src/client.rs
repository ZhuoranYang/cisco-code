//! API client trait and Anthropic implementation.

use anyhow::Result;
use cisco_code_protocol::{StreamEvent, ToolDefinition};

/// The core API client trait.
///
/// Design insight from Claw-Code-Parity: Making the runtime generic over
/// `P: Provider` enables testability (mock providers) and runtime backend
/// swapping (Anthropic ↔ OpenAI ↔ local).
#[allow(async_fn_in_trait)]
pub trait Provider: Send + Sync {
    /// Send a completion request and receive streaming events.
    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<Box<dyn futures::Stream<Item = Result<StreamEvent>> + Send + Unpin>>;
}

/// A completion request to an LLM provider.
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub model: String,
    pub system_prompt: String,
    pub messages: Vec<cisco_code_protocol::Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: u32,
    pub temperature: Option<f64>,
}
