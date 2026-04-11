//! The core conversation runtime — the heart of cisco-code.
//!
//! Design insight from Claw-Code-Parity: `ConversationRuntime<P>` is generic
//! over Provider, making it testable and composable.
//!
//! Design insight from Claude Code: The turn loop yields StreamEvents for
//! real-time TUI rendering (generator pattern). Tool results feed back into
//! the conversation for multi-turn tool use chains.
//!
//! The agent loop:
//! 1. Build CompletionRequest from session + config + tools
//! 2. Call provider.stream(request) → Vec<AssistantEvent>
//! 3. Collect text deltas, emit them as StreamEvents
//! 4. For each ToolUse: check permissions → execute → collect result
//! 5. Add assistant message + tool results to session
//! 6. If stop_reason == tool_use, loop back to step 1
//! 7. If stop_reason == end_turn or budget exhausted, return

use std::sync::Arc;

use anyhow::Result;
use cisco_code_api::{ApiMessage, AssistantEvent, CompletionRequest, Provider};
use cisco_code_protocol::{
    ContentBlock, Message, StopReason, StreamEvent, TokenUsage,
    AssistantMessage, ToolResultMessage,
};
use cisco_code_tools::{ToolContext, ToolRegistry};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::compact::{Compactor, PostCompactRestoration};
use crate::config::RuntimeConfig;
use crate::hooks::{HookEvent, HookInput, HookResult, HookRunner};
use crate::permissions::{PermissionDecision, PermissionEngine};
use crate::prompt::{
    create_scratchpad, detect_git_context, discover_skills, load_memory_content,
    load_project_instructions, resolve_skill, PromptBuilder, SkillContext,
};
use crate::session::Session;
use crate::store::Store;

/// Trait for resolving permission requests interactively.
///
/// The CLI implements this via TUI prompts; the server implements it via
/// WebSocket messages. In headless/SDK mode, no resolver is set and
/// permission requests are auto-denied for safety.
#[async_trait::async_trait]
pub trait PermissionResolver: Send + Sync {
    /// Ask the user whether to allow a tool execution.
    /// Returns `true` if approved, `false` if denied.
    async fn resolve(
        &self,
        tool_use_id: &str,
        tool_name: &str,
        input_summary: &str,
    ) -> bool;
}

/// The core conversation runtime.
///
/// Generic over `P: Provider` (LLM backend) for testability and swappability.
pub struct ConversationRuntime<P: Provider> {
    pub provider: P,
    pub tools: ToolRegistry,
    pub session: Session,
    pub config: RuntimeConfig,
    pub permissions: PermissionEngine,
    pub hooks: HookRunner,
    compactor: Compactor,
    turn_count: u32,
    total_usage: TokenUsage,
    /// Per-session scratchpad directory (created lazily).
    scratchpad_dir: Option<String>,
    /// Optional persistent store for dual-writing messages.
    /// When `Some`, messages are written to the store in addition to the
    /// in-memory session (and the existing JSONL file). When `None`, the
    /// runtime behaves exactly as before — no persistence change.
    store: Option<Arc<dyn Store>>,
    /// Pending system reminders to inject into the next API request.
    /// These are appended as `<system-reminder>` tags to the last user
    /// message in `build_api_messages()`, then drained.
    pending_reminders: Vec<String>,
    /// Optional interactive permission resolver. When `None`, permission
    /// requests that require user approval are auto-denied (safe default).
    permission_resolver: Option<Arc<dyn PermissionResolver>>,
    /// Working directory for tool execution. Defaults to ".".
    working_dir: String,
}

impl<P: Provider> ConversationRuntime<P> {
    pub fn new(provider: P, tools: ToolRegistry, config: RuntimeConfig) -> Self {
        let permissions = PermissionEngine::new(config.permission_mode.clone());
        let hooks = HookRunner::new(".");
        let compactor = Compactor::for_model(&config.model);
        let session = Session::new();
        let scratchpad_dir = create_scratchpad(&session.id);
        Self {
            provider,
            tools,
            session,
            config,
            permissions,
            hooks,
            compactor,
            turn_count: 0,
            total_usage: TokenUsage::default(),
            scratchpad_dir,
            store: None,
            pending_reminders: Vec::new(),
            permission_resolver: None,
            working_dir: ".".to_string(),
        }
    }

    /// Create a runtime with an existing session (for session resume).
    pub fn with_session(provider: P, tools: ToolRegistry, config: RuntimeConfig, session: Session) -> Self {
        let permissions = PermissionEngine::new(config.permission_mode.clone());
        let hooks = HookRunner::new(".");
        let compactor = Compactor::for_model(&config.model);
        let scratchpad_dir = create_scratchpad(&session.id);
        Self {
            provider,
            tools,
            session,
            config,
            permissions,
            hooks,
            compactor,
            turn_count: 0,
            total_usage: TokenUsage::default(),
            scratchpad_dir,
            store: None,
            pending_reminders: Vec::new(),
            permission_resolver: None,
            working_dir: ".".to_string(),
        }
    }

    /// Create a runtime backed by a persistent `Store`.
    ///
    /// If `session_id` is `Some`, loads the existing session and its messages
    /// from the store. If `None`, creates a new session and persists it.
    ///
    /// Messages are dual-written: both to the in-memory `Session` (and its
    /// JSONL log) and to the store. This keeps backward compatibility while
    /// enabling database-backed persistence for daemon/server mode.
    pub async fn with_store(
        provider: P,
        tools: ToolRegistry,
        config: RuntimeConfig,
        store: Arc<dyn Store>,
        session_id: Option<&str>,
    ) -> Result<Self> {
        let session = if let Some(id) = session_id {
            // Resume: load session header + messages from store
            let stored = store
                .get_session(id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("session not found: {id}"))?;
            let messages = store.get_messages(id).await?;
            let mut s = Session::new();
            s.id = stored.id;
            s.metadata = stored.metadata;
            s.messages = messages;
            s
        } else {
            // New session: create in store
            let s = Session::new();
            let stored = crate::store::StoredSession {
                id: s.id.clone(),
                user_id: "local".into(),
                metadata: s.metadata.clone(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            store.create_session(&stored).await?;
            s
        };

        let permissions = PermissionEngine::new(config.permission_mode.clone());
        let hooks = HookRunner::new(".");
        let compactor = Compactor::for_model(&config.model);
        let scratchpad_dir = create_scratchpad(&session.id);

        Ok(Self {
            provider,
            tools,
            session,
            config,
            permissions,
            hooks,
            compactor,
            turn_count: 0,
            total_usage: TokenUsage::default(),
            scratchpad_dir,
            store: Some(store),
            pending_reminders: Vec::new(),
            permission_resolver: None,
            working_dir: ".".to_string(),
        })
    }

    /// Set a persistent store on an existing runtime.
    pub fn set_store(&mut self, store: Arc<dyn Store>) {
        self.store = Some(store);
    }

    /// Set an interactive permission resolver for this runtime.
    ///
    /// When set, the runtime will ask the resolver for approval before
    /// executing dangerous commands, writing to sensitive paths, or when
    /// the permission engine returns `Ask`. Without a resolver, these
    /// cases are auto-denied for safety.
    pub fn set_permission_resolver(&mut self, resolver: Arc<dyn PermissionResolver>) {
        self.permission_resolver = Some(resolver);
    }

    /// Persist a message to the store (if present). Logs errors but does not
    /// fail — store write failures are non-fatal to the agent loop.
    async fn persist_message(&self, message: &Message) {
        if let Some(ref store) = self.store {
            if let Err(e) = store.append_message(&self.session.id, message).await {
                tracing::warn!("Failed to persist message to store: {e}");
            }
        }
    }

    /// Persist updated session metadata to the store (if present).
    async fn persist_metadata(&self) {
        if let Some(ref store) = self.store {
            if let Err(e) = store
                .update_metadata(&self.session.id, &self.session.metadata)
                .await
            {
                tracing::warn!("Failed to persist metadata to store: {e}");
            }
        }
    }

    /// Queue a system reminder to inject into the next API request.
    ///
    /// The content is wrapped in `<system-reminder>` tags and appended to the
    /// last user message when building the API request. Reminders are drained
    /// after each request so they appear exactly once.
    ///
    /// Use cases: task reminders, skill notifications, hook-injected context,
    /// MCP server status, post-compaction context restoration notes.
    pub fn inject_system_reminder(&mut self, content: &str) {
        self.pending_reminders.push(content.to_string());
    }

    /// Get the current working directory for tool execution.
    fn cwd(&self) -> &str {
        &self.working_dir
    }

    /// Set the working directory for tool execution.
    pub fn set_cwd(&mut self, cwd: impl Into<String>) {
        self.working_dir = cwd.into();
    }

    /// Build a PromptBuilder configured for this turn.
    ///
    /// Assembles layered context: core identity, tool guidelines, project
    /// instructions (Agents.md / CLAUDE.md / cisco-code.md), git context,
    /// scratchpad, skills, and memory.
    fn build_prompt(&self) -> PromptBuilder {
        let cwd = self.cwd().to_string();
        let mut builder = PromptBuilder::new(&cwd)
            .with_model(&self.config.model)
            .with_git_context(detect_git_context(&cwd));

        if let Some(instructions) = load_project_instructions(&cwd) {
            builder = builder.with_instructions(instructions);
        }

        if let Some(memory) = load_memory_content(&cwd) {
            builder = builder.with_memory(memory);
        }

        if let Some(ref dir) = self.scratchpad_dir {
            builder = builder.with_scratchpad(dir);
        }

        // Discover and inject available skills
        let skills = discover_skills(&cwd);
        if !skills.is_empty() {
            let skills_text: Vec<String> = skills
                .iter()
                .filter(|s| s.user_invocable)
                .map(|s| {
                    if s.description.is_empty() {
                        format!("- {}", s.name)
                    } else {
                        format!("- {}: {}", s.name, s.description)
                    }
                })
                .collect();
            if !skills_text.is_empty() {
                builder = builder.with_skills(skills_text.join("\n"));
            }
        }

        builder
    }

    /// Convert session messages to API format.
    ///
    /// The Anthropic Messages API expects alternating user/assistant messages.
    /// Tool results must be sent as user messages with tool_result content blocks.
    /// Any pending system reminders are appended to the last user-role message.
    fn build_api_messages(&mut self) -> Vec<ApiMessage> {
        let mut api_messages = Vec::new();

        for msg in &self.session.messages {
            match msg {
                Message::User(user_msg) => {
                    let content: Vec<serde_json::Value> = user_msg
                        .content
                        .iter()
                        .map(|block| match block {
                            ContentBlock::Text { text } => {
                                serde_json::json!({"type": "text", "text": text})
                            }
                            _ => serde_json::json!({"type": "text", "text": ""}),
                        })
                        .collect();

                    api_messages.push(ApiMessage {
                        role: "user".into(),
                        content: serde_json::json!(content),
                    });
                }

                Message::Assistant(asst_msg) => {
                    let content: Vec<serde_json::Value> = asst_msg
                        .content
                        .iter()
                        .map(|block| match block {
                            ContentBlock::Text { text } => {
                                serde_json::json!({"type": "text", "text": text})
                            }
                            ContentBlock::ToolUse { id, name, input } => {
                                serde_json::json!({
                                    "type": "tool_use",
                                    "id": id,
                                    "name": name,
                                    "input": input,
                                })
                            }
                            _ => serde_json::json!({"type": "text", "text": ""}),
                        })
                        .collect();

                    api_messages.push(ApiMessage {
                        role: "assistant".into(),
                        content: serde_json::json!(content),
                    });
                }

                Message::ToolResult(result_msg) => {
                    // Tool results go as user messages with tool_result content blocks
                    // (Anthropic API requirement)
                    let content = serde_json::json!([{
                        "type": "tool_result",
                        "tool_use_id": result_msg.tool_use_id,
                        "content": result_msg.content,
                        "is_error": result_msg.is_error,
                    }]);

                    // Check if last api_message is also a user role (batch tool results)
                    if let Some(last) = api_messages.last_mut() {
                        if last.role == "user" {
                            // Append to existing user message's content array
                            if let Some(arr) = last.content.as_array_mut() {
                                if let Some(new_block) = content.as_array() {
                                    arr.extend(new_block.iter().cloned());
                                    continue;
                                }
                            }
                        }
                    }

                    api_messages.push(ApiMessage {
                        role: "user".into(),
                        content,
                    });
                }

                // System messages and ToolUse messages are handled separately
                // (system goes in system_prompt, tool_use is part of assistant messages)
                _ => {}
            }
        }

        // Inject pending system reminders into the last user-role message.
        // Each reminder is wrapped in <system-reminder> tags so the model
        // can distinguish them from regular content.
        if !self.pending_reminders.is_empty() {
            let reminder_text = self
                .pending_reminders
                .drain(..)
                .map(|r| format!("<system-reminder>\n{r}\n</system-reminder>"))
                .collect::<Vec<_>>()
                .join("\n");

            // Find the last user-role message and append the reminders
            if let Some(last_user) = api_messages.iter_mut().rev().find(|m| m.role == "user") {
                if let Some(arr) = last_user.content.as_array_mut() {
                    arr.push(serde_json::json!({
                        "type": "text",
                        "text": reminder_text,
                    }));
                } else if let Some(text) = last_user.content.as_str() {
                    // Single-string content — convert to array
                    last_user.content = serde_json::json!([
                        {"type": "text", "text": text},
                        {"type": "text", "text": reminder_text},
                    ]);
                }
            } else {
                tracing::warn!(
                    "System reminders dropped: no user message to attach to ({} reminders)",
                    reminder_text.matches("<system-reminder>").count()
                );
            }
        }

        api_messages
    }

    /// Submit a user message and execute the agent loop.
    ///
    /// Returns all events after completion. For real-time streaming, use
    /// `submit_message_streaming` instead.
    pub async fn submit_message(&mut self, user_input: &str) -> Result<Vec<StreamEvent>> {
        self.run_agent_loop(user_input, None).await
    }

    /// Submit a user message with real-time event streaming.
    ///
    /// Events are sent to `event_tx` as they are produced AND collected
    /// into the returned Vec. The sender is best-effort — if the receiver
    /// is dropped mid-stream, the agent loop continues to completion.
    pub async fn submit_message_streaming(
        &mut self,
        user_input: &str,
        event_tx: mpsc::Sender<StreamEvent>,
    ) -> Result<Vec<StreamEvent>> {
        self.run_agent_loop(user_input, Some(event_tx)).await
    }

    /// Core agent loop. Events are always collected; optionally streamed via `event_tx`.
    async fn run_agent_loop(
        &mut self,
        user_input: &str,
        event_tx: Option<mpsc::Sender<StreamEvent>>,
    ) -> Result<Vec<StreamEvent>> {
        // Fire UserPromptSubmit hook before processing.
        // The hook can suppress the prompt (exit 1) or modify it (stdout JSON
        // with "prompt" key or modified input).
        let prompt_hook_input = HookInput {
            event: HookEvent::UserPromptSubmit,
            session_id: self.session.id.clone(),
            tool_name: None,
            tool_input: None,
            tool_result: None,
            is_error: None,
            subagent_id: None,
            stop_reason: None,
            notification: None,
            file_path: None,
            file_operation: None,
            prompt: Some(user_input.to_string()),
            summary_tokens: None,
        };
        let effective_prompt = match self.hooks.run(&prompt_hook_input).await {
            HookResult::Suppress { message } => {
                tracing::info!("User prompt suppressed by hook: {message}");
                return Ok(vec![StreamEvent::Error {
                    message: format!("Prompt blocked by hook: {message}"),
                    recoverable: true,
                }]);
            }
            HookResult::ModifiedInput(modified) => {
                // Hook rewrote the prompt — use the modified version
                modified["prompt"]
                    .as_str()
                    .unwrap_or(user_input)
                    .to_string()
            }
            _ => user_input.to_string(),
        };

        // Add user message to session (using effective prompt after hook processing)
        let user_msg = Message::User(cisco_code_protocol::UserMessage {
            id: Uuid::new_v4(),
            content: vec![ContentBlock::Text {
                text: effective_prompt,
            }],
            attachments: None,
        });
        self.session.add_message(user_msg.clone());
        self.persist_message(&user_msg).await;

        let mut all_events = Vec::new();
        let mut stop_reason = StopReason::ToolUse;

        // Helper closure: push to vec + send to channel (best-effort)
        macro_rules! emit {
            ($event:expr) => {{
                let ev = $event;
                if let Some(ref tx) = event_tx {
                    let _ = tx.try_send(ev.clone());
                }
                all_events.push(ev);
            }};
        }

        // Track whether the loop was terminated early (break). If not, the
        // agent completed normally (model stopped calling tools).
        let mut early_exit = false;

        // Agent loop: continue until model stops calling tools
        while stop_reason == StopReason::ToolUse {
            self.turn_count += 1;

            emit!(StreamEvent::TurnStart {
                model: self.config.model.clone(),
                turn_number: self.turn_count,
            });

            // 1. Build the completion request
            let prompt = self.build_prompt();
            let system_blocks = prompt.build_system_blocks();
            let request = CompletionRequest {
                model: self.config.model.clone(),
                system_prompt: prompt.build(),
                messages: self.build_api_messages(),
                tools: self.tools.definitions(),
                max_tokens: self.config.max_tokens,
                temperature: self.config.temperature,
                thinking: self.config.thinking.clone(),
                system_blocks: Some(system_blocks),
            };

            // 2. Call the provider
            let assistant_events = self.provider.stream(request).await?;

            // 3. Process events: collect text, tool uses, and stop reason
            let mut text_parts = Vec::new();
            let mut tool_uses: Vec<(String, String, serde_json::Value)> = Vec::new(); // (id, name, input)
            let mut turn_usage = TokenUsage::default();
            stop_reason = StopReason::EndTurn; // default if no explicit stop

            for event in &assistant_events {
                match event {
                    AssistantEvent::TextDelta(text) => {
                        emit!(StreamEvent::TextDelta {
                            text: text.clone(),
                        });
                        text_parts.push(text.clone());
                    }
                    AssistantEvent::ThinkingDelta(_thinking) => {
                        // Extended thinking content — collected for display but not
                        // added to conversation history (model's internal reasoning).
                    }
                    AssistantEvent::ToolUse { id, name, input } => {
                        emit!(StreamEvent::ToolUseStart {
                            tool_use_id: id.clone(),
                            tool_name: name.clone(),
                        });
                        tool_uses.push((id.clone(), name.clone(), input.clone()));
                    }
                    AssistantEvent::Usage {
                        input_tokens,
                        output_tokens,
                    } => {
                        turn_usage.input_tokens += input_tokens;
                        turn_usage.output_tokens += output_tokens;
                    }
                    AssistantEvent::MessageStop {
                        stop_reason: reason,
                    } => {
                        stop_reason = match reason.as_str() {
                            "tool_use" => StopReason::ToolUse,
                            "end_turn" => StopReason::EndTurn,
                            "max_tokens" => StopReason::MaxTokens,
                            "stop_sequence" => StopReason::StopSequence,
                            _ => StopReason::EndTurn,
                        };
                    }
                }
            }

            // 4. Build assistant message content blocks
            let mut assistant_blocks = Vec::new();
            let full_text: String = text_parts.concat();
            if !full_text.is_empty() {
                assistant_blocks.push(ContentBlock::Text { text: full_text });
            }
            for (id, name, input) in &tool_uses {
                assistant_blocks.push(ContentBlock::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: input.clone(),
                });
            }

            // 5. Add assistant message to session
            let assistant_msg = Message::Assistant(AssistantMessage {
                id: Uuid::new_v4(),
                content: assistant_blocks,
                model: self.config.model.clone(),
                usage: turn_usage.clone(),
                stop_reason: Some(stop_reason.clone()),
            });
            self.session.add_message(assistant_msg.clone());
            self.persist_message(&assistant_msg).await;

            // 6. Execute tools and add results to session
            if stop_reason == StopReason::ToolUse {
                let tool_ctx = ToolContext {
                    cwd: self.cwd().to_string(),
                    interactive: true,
                    progress_tx: event_tx.clone(),
                };

                for (tool_id, tool_name, tool_input) in &tool_uses {
                    // 6a. Run pre-tool hooks (before permissions — hooks can approve/deny)
                    let mut skip_permission_check = false;
                    let hook_input = HookInput {
                        event: HookEvent::PreToolUse,
                        session_id: self.session.id.clone(),
                        tool_name: Some(tool_name.clone()),
                        tool_input: Some(tool_input.clone()),
                        tool_result: None,
                        is_error: None,
                        subagent_id: None,
                        stop_reason: None,
                        notification: None,
                        file_path: None,
                        file_operation: None,
                        prompt: None,
                        summary_tokens: None,
                    };

                    let mut effective_input = tool_input.clone();
                    match self.hooks.run(&hook_input).await {
                        HookResult::Suppress { message } => {
                            emit!(StreamEvent::ToolResult {
                                tool_use_id: tool_id.clone(),
                                result: format!("Blocked by hook: {message}"),
                                is_error: true,
                            });
                            let result_msg = Message::ToolResult(ToolResultMessage {
                                id: Uuid::new_v4(),
                                tool_use_id: tool_id.clone(),
                                content: format!("Blocked by hook: {message}"),
                                is_error: true,
                                injected_messages: None,
                            });
                            self.session.add_message(result_msg.clone());
                            self.persist_message(&result_msg).await;
                            continue;
                        }
                        HookResult::Approve => {
                            // Hook explicitly approved — skip the permission engine
                            skip_permission_check = true;
                        }
                        HookResult::ApproveWithModifiedInput(modified) => {
                            // Hook approved AND modified the input
                            skip_permission_check = true;
                            effective_input = modified;
                        }
                        HookResult::Deny { reason } => {
                            emit!(StreamEvent::ToolResult {
                                tool_use_id: tool_id.clone(),
                                result: format!("Denied by hook: {reason}"),
                                is_error: true,
                            });
                            let result_msg = Message::ToolResult(ToolResultMessage {
                                id: Uuid::new_v4(),
                                tool_use_id: tool_id.clone(),
                                content: format!("Denied by hook: {reason}"),
                                is_error: true,
                                injected_messages: None,
                            });
                            self.session.add_message(result_msg.clone());
                            self.persist_message(&result_msg).await;
                            continue;
                        }
                        HookResult::ModifiedInput(modified) => {
                            effective_input = modified;
                        }
                        HookResult::Error { command, message } => {
                            // If HookRunner::run() propagated the error, it was
                            // from a required hook — block tool execution.
                            emit!(StreamEvent::ToolResult {
                                tool_use_id: tool_id.clone(),
                                result: format!(
                                    "Required hook '{command}' failed: {message}"
                                ),
                                is_error: true,
                            });
                            let result_msg = Message::ToolResult(ToolResultMessage {
                                id: Uuid::new_v4(),
                                tool_use_id: tool_id.clone(),
                                content: format!(
                                    "Required hook '{command}' failed: {message}"
                                ),
                                is_error: true,
                                injected_messages: None,
                            });
                            self.session.add_message(result_msg.clone());
                            self.persist_message(&result_msg).await;
                            continue;
                        }
                        HookResult::Continue => {}
                    }

                    // 6b. Safety checks + permission check.
                    // Dangerous command and sensitive path checks are security
                    // invariants that ALWAYS run — even when a hook approved.
                    // Only the mode-based permission decision is skippable.
                    {
                        let input_summary =
                            summarize_tool_input(tool_name, &effective_input);

                        // Hard safety checks (never skipped).
                        // These BLOCK execution until resolved — either via the
                        // interactive permission resolver or auto-deny in headless mode.
                        let mut safety_denied = false;
                        if tool_name == "Bash" {
                            if let Some(warning) =
                                crate::permissions::detect_dangerous_command(
                                    &input_summary,
                                )
                            {
                                let summary = format!(
                                    "⚠ {warning}. Command: {input_summary}"
                                );
                                emit!(StreamEvent::PermissionRequest {
                                    tool_use_id: tool_id.clone(),
                                    tool_name: tool_name.clone(),
                                    input_summary: summary.clone(),
                                });
                                let approved = match &self.permission_resolver {
                                    Some(resolver) => {
                                        resolver.resolve(&tool_id, tool_name, &summary).await
                                    }
                                    None => false, // auto-deny in headless mode
                                };
                                if !approved {
                                    safety_denied = true;
                                }
                            }
                        }
                        if !safety_denied && matches!(tool_name.as_str(), "Write" | "Edit") {
                            if let Some(warning) =
                                crate::permissions::detect_sensitive_path(
                                    &input_summary,
                                )
                            {
                                let summary = format!(
                                    "⚠ {warning}: {input_summary}"
                                );
                                emit!(StreamEvent::PermissionRequest {
                                    tool_use_id: tool_id.clone(),
                                    tool_name: tool_name.clone(),
                                    input_summary: summary.clone(),
                                });
                                let approved = match &self.permission_resolver {
                                    Some(resolver) => {
                                        resolver.resolve(&tool_id, tool_name, &summary).await
                                    }
                                    None => false, // auto-deny in headless mode
                                };
                                if !approved {
                                    safety_denied = true;
                                }
                            }
                        }
                        if safety_denied {
                            let reason = "Blocked by safety check: user denied or no interactive resolver";
                            self.permissions
                                .denial_tracker_mut()
                                .record_denial(tool_name);
                            emit!(StreamEvent::ToolResult {
                                tool_use_id: tool_id.clone(),
                                result: format!("Permission denied: {reason}"),
                                is_error: true,
                            });
                            let result_msg = Message::ToolResult(ToolResultMessage {
                                id: Uuid::new_v4(),
                                tool_use_id: tool_id.clone(),
                                content: format!("Permission denied: {reason}"),
                                is_error: true,
                                injected_messages: None,
                            });
                            self.session.add_message(result_msg.clone());
                            self.persist_message(&result_msg).await;
                            continue;
                        }

                        // Mode-based permission check (skipped if hook approved)
                        if !skip_permission_check {
                            let permission_level = self
                                .tools
                                .get(tool_name)
                                .map(|t| t.permission_level())
                                .unwrap_or(
                                    cisco_code_protocol::PermissionLevel::Execute,
                                );

                            match self.permissions.check(
                                tool_name,
                                permission_level,
                                &input_summary,
                            ) {
                                PermissionDecision::Allow => {} // proceed
                                PermissionDecision::Ask { reason: _ } => {
                                    emit!(StreamEvent::PermissionRequest {
                                        tool_use_id: tool_id.clone(),
                                        tool_name: tool_name.clone(),
                                        input_summary: input_summary.clone(),
                                    });
                                    // Ask the permission resolver; auto-deny in headless mode
                                    let approved = match &self.permission_resolver {
                                        Some(resolver) => {
                                            resolver
                                                .resolve(&tool_id, tool_name, &input_summary)
                                                .await
                                        }
                                        None => false,
                                    };
                                    if approved {
                                        // Record session approval so we don't ask again
                                        self.permissions.approve_specific(
                                            tool_name,
                                            &input_summary,
                                        );
                                    } else {
                                        self.permissions
                                            .denial_tracker_mut()
                                            .record_denial(tool_name);
                                        emit!(StreamEvent::ToolResult {
                                            tool_use_id: tool_id.clone(),
                                            result: "Permission denied by user".to_string(),
                                            is_error: true,
                                        });
                                        let result_msg =
                                            Message::ToolResult(ToolResultMessage {
                                                id: Uuid::new_v4(),
                                                tool_use_id: tool_id.clone(),
                                                content: "Permission denied by user"
                                                    .to_string(),
                                                is_error: true,
                                                injected_messages: None,
                                            });
                                        self.session.add_message(result_msg.clone());
                                        self.persist_message(&result_msg).await;
                                        continue;
                                    }
                                }
                                PermissionDecision::Deny { reason } => {
                                    self.permissions
                                        .denial_tracker_mut()
                                        .record_denial(tool_name);
                                    emit!(StreamEvent::ToolResult {
                                        tool_use_id: tool_id.clone(),
                                        result: format!(
                                            "Permission denied: {reason}"
                                        ),
                                        is_error: true,
                                    });
                                    let result_msg =
                                        Message::ToolResult(ToolResultMessage {
                                            id: Uuid::new_v4(),
                                            tool_use_id: tool_id.clone(),
                                            content: format!(
                                                "Permission denied: {reason}"
                                            ),
                                            is_error: true,
                                            injected_messages: None,
                                        });
                                    self.session.add_message(result_msg.clone());
                                    self.persist_message(&result_msg).await;
                                    continue;
                                }
                            }
                        }
                    }

                    // 6c. Execute the tool
                    emit!(StreamEvent::ToolExecutionStart {
                        tool_use_id: tool_id.clone(),
                        tool_name: tool_name.clone(),
                        description: format!("Executing {tool_name}"),
                    });

                    // Clone before moving into execute() — needed for post-tool hooks
                    let effective_input_snapshot = effective_input.clone();
                    let result = self
                        .tools
                        .execute(tool_name, effective_input, &tool_ctx)
                        .await;

                    let tool_result = match result {
                        Ok(r) => r,
                        Err(e) => cisco_code_protocol::ToolResult::error(format!(
                            "Tool execution failed: {e}"
                        )),
                    };

                    // 6d. Run post-tool hooks
                    let post_hook_input = HookInput {
                        event: HookEvent::PostToolUse,
                        session_id: self.session.id.clone(),
                        tool_name: Some(tool_name.clone()),
                        tool_input: Some(effective_input_snapshot.clone()),
                        tool_result: Some(tool_result.output.clone()),
                        is_error: Some(tool_result.is_error),
                        subagent_id: None,
                        stop_reason: None,
                        notification: None,
                        file_path: None,
                        file_operation: None,
                        prompt: None,
                        summary_tokens: None,
                    };
                    let _ = self.hooks.run(&post_hook_input).await;

                    // 6e. Fire FileChanged hook for file-mutating tools
                    if !tool_result.is_error
                        && matches!(
                            tool_name.as_str(),
                            "Write" | "Edit" | "ApplyPatch"
                        )
                    {
                        let file_path = effective_input_snapshot["file_path"]
                            .as_str()
                            .map(|s| s.to_string());
                        let file_op = match tool_name.as_str() {
                            "Write" => "write",
                            "Edit" => "edit",
                            "ApplyPatch" => "patch",
                            _ => "unknown",
                        };
                        let file_hook_input = HookInput {
                            event: HookEvent::FileChanged,
                            session_id: self.session.id.clone(),
                            tool_name: Some(tool_name.clone()),
                            tool_input: None,
                            tool_result: None,
                            is_error: None,
                            subagent_id: None,
                            stop_reason: None,
                            notification: None,
                            file_path,
                            file_operation: Some(file_op.to_string()),
                            prompt: None,
                            summary_tokens: None,
                        };
                        let _ = self.hooks.run(&file_hook_input).await;
                    }

                    // Check if this is a Skill tool invocation that needs expansion
                    let tool_result = if tool_name == "Skill" && !tool_result.is_error {
                        self.expand_skill_result(tool_result)
                    } else {
                        tool_result
                    };

                    // Emit tool result event
                    emit!(StreamEvent::ToolResult {
                        tool_use_id: tool_id.clone(),
                        result: tool_result.output.clone(),
                        is_error: tool_result.is_error,
                    });

                    // Add tool result message to session
                    let result_msg = Message::ToolResult(ToolResultMessage {
                        id: Uuid::new_v4(),
                        tool_use_id: tool_id.clone(),
                        content: tool_result.output,
                        is_error: tool_result.is_error,
                        injected_messages: tool_result.injected_messages,
                    });
                    self.session.add_message(result_msg.clone());
                    self.persist_message(&result_msg).await;
                }
            }

            // 7. Update cumulative usage and persist session metadata
            self.total_usage.merge(&turn_usage);

            let estimated_cost = (self.total_usage.input_tokens as f64 * 3.0
                + self.total_usage.output_tokens as f64 * 15.0)
                / 1_000_000.0;
            self.session.update_usage(&self.total_usage, estimated_cost, self.turn_count);
            self.persist_metadata().await;

            emit!(StreamEvent::TurnEnd {
                stop_reason: stop_reason.clone(),
                usage: turn_usage,
            });

            // Safety: prevent infinite loops
            if self.turn_count >= self.config.max_turns {
                emit!(StreamEvent::Error {
                    message: format!("Turn limit reached ({})", self.config.max_turns),
                    recoverable: false,
                });
                let stop_input = HookInput {
                    event: HookEvent::Stop,
                    session_id: self.session.id.clone(),
                    tool_name: None,
                    tool_input: None,
                    tool_result: None,
                    is_error: None,
                    subagent_id: None,
                    stop_reason: Some("max_turns".to_string()),
                    notification: None,
                    file_path: None,
                    file_operation: None,
                    prompt: None,
                    summary_tokens: None,
                };
                let _ = self.hooks.run(&stop_input).await;
                early_exit = true;
                break;
            }

            // Budget check (using the cost already computed for metadata)
            if let Some(budget) = self.config.max_budget_usd {
                if estimated_cost > budget {
                    emit!(StreamEvent::Error {
                        message: format!(
                            "Budget limit reached (${:.2} > ${:.2})",
                            estimated_cost, budget
                        ),
                        recoverable: false,
                    });
                    let stop_input = HookInput {
                        event: HookEvent::Stop,
                        session_id: self.session.id.clone(),
                        tool_name: None,
                        tool_input: None,
                        tool_result: None,
                        is_error: None,
                        subagent_id: None,
                        stop_reason: Some("budget_exceeded".to_string()),
                        notification: None,
                        file_path: None,
                        file_operation: None,
                        prompt: None,
                        summary_tokens: None,
                    };
                    let _ = self.hooks.run(&stop_input).await;
                    early_exit = true;
                    break;
                }
            }

            // Context compaction check
            self.compactor.update_estimate(&self.session.messages);
            if self.compactor.needs_compaction() {
                let pre_compact_count = self.session.messages.len();
                tracing::info!(
                    "Context compaction triggered ({} estimated tokens, {} messages)",
                    self.compactor.estimated_tokens(),
                    pre_compact_count,
                );

                // Fire CompactionStart hook
                let compact_start = HookInput {
                    event: HookEvent::CompactionStart,
                    session_id: self.session.id.clone(),
                    tool_name: None,
                    tool_input: None,
                    tool_result: None,
                    is_error: None,
                    subagent_id: None,
                    stop_reason: None,
                    notification: None,
                    file_path: None,
                    file_operation: None,
                    prompt: None,
                    summary_tokens: None,
                };
                let _ = self.hooks.run(&compact_start).await;

                // Collect recently-referenced file paths BEFORE compaction
                // (avoids cloning the entire message list — just the paths).
                let restoration_config = PostCompactRestoration::default();
                let recent_file_paths = crate::compact::collect_recent_files(
                    &self.session.messages,
                    restoration_config.max_files,
                );

                match self
                    .compactor
                    .compact(
                        &self.session.messages,
                        &self.provider,
                        &self.config.model,
                    )
                    .await
                {
                    Ok(compacted) => {
                        let compacted_count = pre_compact_count - compacted.len();
                        self.session.messages = compacted;

                        // Write a compact boundary marker to the session
                        let boundary = Message::CompactBoundary(
                            cisco_code_protocol::CompactBoundaryMessage {
                                id: Uuid::new_v4(),
                                summary: format!(
                                    "Context compacted: {compacted_count} messages summarized"
                                ),
                                compacted_message_count: compacted_count,
                                timestamp: chrono::Utc::now().to_rfc3339(),
                            },
                        );
                        self.session.add_message(boundary.clone());
                        self.persist_message(&boundary).await;
                        self.session.record_compaction();

                        // Post-compaction file restoration: re-inject recently
                        // referenced file contents so the model retains working
                        // context after the summary replaces raw messages.
                        if let Some(snapshot) =
                            restoration_config.build_from_paths(
                                &recent_file_paths,
                                self.cwd(),
                            )
                        {
                            tracing::info!(
                                "Injecting post-compaction file snapshot ({} chars)",
                                snapshot.len(),
                            );
                            self.inject_system_reminder(&snapshot);
                        }

                        // Fire CompactionEnd hook
                        let summary_tokens = self.compactor.estimated_tokens();
                        let compact_end = HookInput {
                            event: HookEvent::CompactionEnd,
                            session_id: self.session.id.clone(),
                            tool_name: None,
                            tool_input: None,
                            tool_result: None,
                            is_error: None,
                            subagent_id: None,
                            stop_reason: None,
                            notification: None,
                            file_path: None,
                            file_operation: None,
                            prompt: None,
                            summary_tokens: Some(summary_tokens),
                        };
                        let _ = self.hooks.run(&compact_end).await;
                    }
                    Err(e) => {
                        tracing::warn!("Context compaction failed: {e}");
                        // Continue without compaction — better than crashing
                    }
                }
            }
        }

        // Fire Stop hook for normal completion (early exits already fired their own).
        if !early_exit {
            let stop_input = HookInput {
                event: HookEvent::Stop,
                session_id: self.session.id.clone(),
                tool_name: None,
                tool_input: None,
                tool_result: None,
                is_error: None,
                subagent_id: None,
                stop_reason: Some("completed".to_string()),
                notification: None,
                file_path: None,
                file_operation: None,
                prompt: None,
                summary_tokens: None,
            };
            let _ = self.hooks.run(&stop_input).await;
        }

        Ok(all_events)
    }

    /// Get cumulative token usage for this session.
    pub fn total_usage(&self) -> &TokenUsage {
        &self.total_usage
    }

    /// Get the number of turns completed.
    pub fn turn_count(&self) -> u32 {
        self.turn_count
    }

    /// Expand a Skill tool result by resolving the skill name and injecting its content.
    ///
    /// When the Skill tool returns `{"action": "invoke_skill", "skill": "commit", ...}`,
    /// we look up the skill definition (bundled or filesystem) and replace the tool result
    /// with the skill's expanded prompt content. This matches Claude Code's behavior where
    /// `/commit` expands into the full commit skill instructions.
    fn expand_skill_result(
        &self,
        mut result: cisco_code_protocol::ToolResult,
    ) -> cisco_code_protocol::ToolResult {
        // Parse the skill invocation descriptor
        let descriptor: serde_json::Value = match serde_json::from_str(&result.output) {
            Ok(v) => v,
            Err(_) => return result,
        };

        let action = descriptor["action"].as_str().unwrap_or("");
        if action != "invoke_skill" {
            return result;
        }

        let skill_name = match descriptor["skill"].as_str() {
            Some(name) => name,
            None => return result,
        };
        let args = descriptor["args"].as_str();

        let cwd = self.cwd().to_string();

        // Resolve the skill from the discovery chain
        match resolve_skill(&cwd, skill_name) {
            Some(skill) => {
                match skill.context {
                    SkillContext::Fork => {
                        // Forked execution: return a JSON descriptor that tells the agent
                        // loop to execute this skill as an isolated sub-agent conversation.
                        // The Agent tool / sub-agent infrastructure handles actual execution.
                        let mut fork_descriptor = serde_json::json!({
                            "action": "fork_skill",
                            "skill": skill.name,
                            "content": format!(
                                "{}\n\n{}",
                                skill.description,
                                skill.content,
                            ),
                        });

                        if let Some(ref model) = skill.model {
                            fork_descriptor["model"] = serde_json::Value::String(model.clone());
                        }

                        if let Some(ref tools) = skill.allowed_tools {
                            fork_descriptor["allowed_tools"] = serde_json::json!(tools);
                        }

                        if let Some(args) = args {
                            fork_descriptor["args"] = serde_json::Value::String(args.to_string());
                        }

                        result.output = fork_descriptor.to_string();
                    }
                    SkillContext::Inline => {
                        // Inline execution: expand skill content directly into the conversation
                        let mut expanded = format!(
                            "<command-name>{}</command-name>\n\n{}\n\n{}",
                            skill.name,
                            skill.description,
                            skill.content,
                        );

                        if let Some(args) = args {
                            expanded.push_str(&format!("\n\nArguments: {args}"));
                        }

                        result.output = expanded;
                    }
                }
            }
            None => {
                result.output = format!(
                    "Unknown skill: '{skill_name}'. Available skills can be listed with /help."
                );
                result.is_error = true;
            }
        }

        result
    }
}

/// Create a short human-readable summary of tool input for permission prompts.
fn summarize_tool_input(tool_name: &str, input: &serde_json::Value) -> String {
    match tool_name {
        "Bash" => input["command"]
            .as_str()
            .unwrap_or("(unknown command)")
            .to_string(),
        "Read" => input["file_path"]
            .as_str()
            .unwrap_or("(unknown file)")
            .to_string(),
        "Write" => {
            let path = input["file_path"].as_str().unwrap_or("(unknown)");
            format!("{path} (write)")
        }
        "Edit" => {
            let path = input["file_path"].as_str().unwrap_or("(unknown)");
            format!("{path} (edit)")
        }
        "Grep" => {
            let pattern = input["pattern"].as_str().unwrap_or("(unknown)");
            format!("grep for '{pattern}'")
        }
        "Glob" => {
            let pattern = input["pattern"].as_str().unwrap_or("(unknown)");
            format!("glob '{pattern}'")
        }
        _ => {
            let s = input.to_string();
            if s.len() > 100 {
                // Find a safe UTF-8 char boundary to avoid panicking on multi-byte chars
                let mut end = 100;
                while end > 0 && !s.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}...", &s[..end])
            } else {
                s
            }
        }
    }
}
