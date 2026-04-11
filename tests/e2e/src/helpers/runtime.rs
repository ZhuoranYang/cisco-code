//! Test runtime construction helpers.

use cisco_code_api::bedrock::BedrockClient;
use cisco_code_runtime::{ConversationRuntime, RuntimeConfig, PermissionMode, SandboxMode};
use cisco_code_tools::ToolRegistry;

use super::provider::{HAIKU_MODEL, E2E_MAX_TOKENS, E2E_TEMPERATURE, E2E_MAX_TURNS};

/// Build a minimal runtime with no tools (text-only conversations).
pub fn minimal_runtime(client: BedrockClient) -> ConversationRuntime<BedrockClient> {
    let tools = ToolRegistry::new();
    let config = RuntimeConfig {
        model: HAIKU_MODEL.to_string(),
        model_class: None,
        max_tokens: E2E_MAX_TOKENS,
        max_turns: E2E_MAX_TURNS,
        max_budget_usd: None,
        temperature: Some(E2E_TEMPERATURE),
        permission_mode: PermissionMode::BypassPermissions,
        sandbox_mode: SandboxMode::None,
        thinking: None,
    };
    ConversationRuntime::new(client, tools, config)
}

/// Build a runtime with specific tools registered, working in the given directory.
pub fn runtime_with_tools(
    client: BedrockClient,
    cwd: &str,
    register_fn: impl FnOnce(&mut ToolRegistry),
) -> ConversationRuntime<BedrockClient> {
    let mut tools = ToolRegistry::new();
    register_fn(&mut tools);
    let config = RuntimeConfig {
        model: HAIKU_MODEL.to_string(),
        model_class: None,
        max_tokens: E2E_MAX_TOKENS,
        max_turns: E2E_MAX_TURNS,
        max_budget_usd: None,
        temperature: Some(E2E_TEMPERATURE),
        permission_mode: PermissionMode::BypassPermissions,
        sandbox_mode: SandboxMode::None,
        thinking: None,
    };
    let mut runtime = ConversationRuntime::new(client, tools, config);
    runtime.set_cwd(cwd);
    runtime
}

/// Build a runtime with custom max_turns.
pub fn runtime_with_max_turns(
    client: BedrockClient,
    max_turns: u32,
    register_fn: impl FnOnce(&mut ToolRegistry),
) -> ConversationRuntime<BedrockClient> {
    let mut tools = ToolRegistry::new();
    register_fn(&mut tools);
    let config = RuntimeConfig {
        model: HAIKU_MODEL.to_string(),
        model_class: None,
        max_tokens: E2E_MAX_TOKENS,
        max_turns,
        max_budget_usd: None,
        temperature: Some(E2E_TEMPERATURE),
        permission_mode: PermissionMode::BypassPermissions,
        sandbox_mode: SandboxMode::None,
        thinking: None,
    };
    ConversationRuntime::new(client, tools, config)
}
