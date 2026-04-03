//! System prompt builder.
//!
//! Design insight from Astro-Assistant: Layered prompt assembly:
//! 1. SYSTEM — core agent instructions
//! 2. CONTEXT — git status, environment, repo structure
//! 3. RUNTIME_STATE — session state, memory injections
//! 4. HISTORY — conversation history (compacted if needed)
//! 5. TOOL_DESCRIPTIONS — available tool schemas
//! 6. INSTRUCTIONS — project-specific (CLAUDE.md / cisco-code.md)
//! 7. REMINDERS — mid-conversation nudges

/// Placeholder for Phase 1 implementation.
pub struct PromptBuilder;
