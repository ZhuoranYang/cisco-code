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

pub mod channels;
pub mod commands;
pub mod compact;
pub mod config;
pub mod conversation;
pub mod cron;
pub mod event_bus;
pub mod hooks;
pub mod memory;
pub mod microcompact;
pub mod notify;
pub mod permissions;
pub mod prompt;
pub mod prompt_sections;
pub mod router;
pub mod session;
pub mod store;
pub mod store_sqlite;
pub mod streaming_executor;
pub mod subagent;
pub mod tasks;
pub mod worktree;

pub use commands::{CommandKind, CommandRegistry, CommandResult, SlashCommand};
pub use compact::{
    collect_recent_files, threshold_for_model, CompactConfig, Compactor, PostCompactRestoration,
};
pub use config::*;
pub use conversation::*;
pub use microcompact::{compaction_level, CompactionLevel, MicroCompactConfig, MicroCompactor};
pub use cron::{CronJob, CronManager, CronSchedule};
pub use event_bus::{event_bus, EventReceiver, EventSender, TriggerEvent};
pub use hooks::{HookConfig, HookEvent, HookInput, HookResult, HookRunner};
pub use memory::{MemoryEntry, MemoryManager, MemoryType};
pub use notify::{Notification, NotificationChannel, NotificationLevel, Notifier};
pub use permissions::{
    detect_dangerous_command, detect_sensitive_path, DenialTracker, PathRule, PermissionDecision,
    PermissionEngine, PermissionOverride, ToolPermissionRule,
};
pub use prompt::{
    create_scratchpad, discover_skills, load_bundled_skills, load_memory_content,
    load_project_instructions, load_settings, resolve_skill, InstructionFile, InstructionSource,
    SkillContext, SkillInfo,
};
pub use prompt_sections::PromptSectionRegistry;
pub use router::SessionRouter;
pub use session::{Session, SessionInfo, SessionMetadata};
pub use store::{SessionSummary, Store, StoredSession};
pub use store_sqlite::SqliteStore;
pub use streaming_executor::{execute_tool_batch, PendingToolCall, ToolCallResult};
pub use subagent::{SubagentConfig, SubagentResult};
pub use tasks::{Task, TaskManager, TaskStatus, TaskSummary};
pub use worktree::{WorktreeInfo, WorktreeManager};
