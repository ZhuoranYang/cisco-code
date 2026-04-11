# Codex — Internal Architecture Analysis

Source: `/Users/zhuoran_cisco/Documents/agent_tools/tmp/codex`
Language: Rust (primary), with TypeScript CLI wrapper
Workspace: `codex-rs/` with crates: `core`, `hooks`, `linux-sandbox`, `mcp-client`, `mcp-types`, `tui`, `exec`

---

## 1. Knowledge Management

### Project Instructions (`codex-rs/core/src/project_doc.rs`)

- **Primary file**: `AGENTS.md` (constant `DEFAULT_PROJECT_DOC_FILENAME`)
- **Local override**: `AGENTS.override.md` (constant `LOCAL_PROJECT_DOC_FILENAME`)
- **Fallback filenames**: configurable via `project_doc_fallback_filenames` in config

Discovery: `discover_project_doc_paths()` walks upward from cwd to project root (detected via `.git`, overridden by `project_root_markers`), collects every `AGENTS.md` along the path, concatenates root-to-cwd order. Budget capped by `project_doc_max_bytes`.

Assembly: `get_user_instructions()` combines:
1. `Config::user_instructions`
2. Project docs (AGENTS.md chain)
3. Optional JS REPL instructions
4. `HIERARCHICAL_AGENTS_MESSAGE` (when `ChildAgentsMd` feature enabled)

Separated by `"\n\n--- project-doc ---\n\n"`.
Injected as structured `UserInstructions` messages wrapped in `AGENTS_MD_FRAGMENT` markers.

### Memory System (`codex-rs/core/src/memories/mod.rs`)

Two-phase memory pipeline via `start_memories_startup_task()`:

- **Phase 1** (`phase1.rs`): extracts raw memories from past rollouts using `gpt-5.1-codex-mini`
- **Phase 2** (`phase2.rs`): consolidates across threads, writes `MEMORY.md` / `memory_summary.md`
- **Storage root**: `$CODEX_HOME/memories/`
- **Artifacts**: `raw_memories.md`, `rollout_summaries/<timestamp>-<hash>-<slug>.md`

### Scratchpad

No dedicated scratchpad feature. The JS REPL (`js_repl` tool) with persistent kernel is the closest analogue.

---

## 2. Session Management

### Storage Format (`codex-rs/core/src/rollout/mod.rs`)

Sessions stored as "rollouts" — JSONL transcript files:
- Active: `$CODEX_HOME/sessions/<thread-id>/`
- Archived: `$CODEX_HOME/archived_sessions/<thread-id>/`
- SQLite state DB at `$CODEX_SQLITE_HOME` via `codex_state::StateRuntime`

### Resume Logic (`codex-rs/core/src/codex.rs`)

`InitialHistory` enum:
- `New` — fresh session
- `Resumed(ResumedHistory { rollout_path, conversation_id })` — loads from JSONL
- `Forked(...)` — branched session

On `Resumed`, history loaded via `RolloutRecorder::get_rollout_history()`.
Hook fires `SessionStartSource::Resume` vs `SessionStartSource::New`.

---

## 3. Context Management

### System Prompt Assembly

`SessionState` owns a `ContextManager` (history: `Vec<ResponseItem>`) and `reference_context_item` for diffing context changes turn-to-turn.

Key files:
- `codex-rs/core/src/state/session.rs`
- `codex-rs/core/src/context_manager/history.rs`

### Token Tracking

- `ContextManager::update_token_info()` tracks per-turn from API responses
- `get_total_token_usage(server_reasoning_included: bool)` aggregates
- `set_token_usage_full(context_window)` marks context full

### Auto-Compaction (`codex-rs/core/src/compact.rs`)

- `run_inline_auto_compact_task()`: local summarization via `SUMMARIZATION_PROMPT`
- `should_use_remote_compact_task()`: routes to remote if provider `is_openai()`
- `InitialContextInjection` enum controls re-injection timing
- `model_auto_compact_token_limit` in config sets threshold

---

## 4. Skills / Plugins

### Skills System (`codex-rs/core/src/skills/`)

Skills are Markdown files with TOML frontmatter loaded by `loader.rs`.
Search path:
1. `$CODEX_HOME/skills/`
2. Project `.codex/skills/`
3. System `/etc/codex/skills/`

Manager: `SkillsManager` in `manager.rs`.
Skills can declare tool dependencies (`SkillToolDependency`) and env var dependencies.

### Built-in Tools

Handlers in `codex-rs/core/src/tools/handlers/`:
`shell`, `read_file`, `list_dir`, `grep_files`, `apply_patch`, `view_image`,
`tool_search`, `tool_suggest`, `unified_exec`, `js_repl`, `plan`,
`multi_agents`, `agent_jobs`, `artifacts`, `request_user_input`,
`request_permissions`, `mcp`, `mcp_resource`

### Hook System (`codex-rs/hooks/src/`)

- `Hook` wraps async `HookFn`
- Events: `AfterAgent`, `AfterToolUse`
- Results: `Success`, `FailedContinue`, `FailedAbort`
- Registry in `registry.rs` with `HooksConfig`

---

## 5. Config

### File Locations

| Layer | Path |
|-------|------|
| System | `/etc/codex/config.toml` |
| User | `$CODEX_HOME/config.toml` (default: `~/.codex/config.toml`) |
| Project | `.codex/config.toml` in project root |
| Managed | `$CODEX_HOME/managed_config.toml` |

Format: TOML.

### Key Config Fields

- `model`, `model_provider`, `model_context_window`
- `model_auto_compact_token_limit`
- `model_reasoning_effort`: `ReasoningEffort` enum
- `openai_base_url`, `chatgpt_base_url`
- `model_providers: HashMap<String, ModelProviderInfo>`

### Environment Variables

- `CODEX_HOME`: override home directory
- `CODEX_SQLITE_HOME`: override SQLite location
- `OPENAI_BASE_URL`: deprecated API base override

Custom prompts: `$CODEX_HOME/prompts/*.md`.

---

## Key Differences from Claude Code

| Feature | Claude Code | Codex |
|---------|-------------|-------|
| Instruction file | CLAUDE.md | AGENTS.md |
| Config format | JSON (settings.json) | TOML (config.toml) |
| Home dir | ~/.claude | ~/.codex |
| Memory | File-based MEMORY.md | LLM-generated from rollouts |
| Scratchpad | Per-session /tmp/claude-<uid>/ | None (JS REPL) |
| Skills format | YAML frontmatter in SKILL.md | TOML frontmatter in SKILL.md |
| Session format | JSONL in project dir | JSONL "rollouts" per thread |
| Compaction | Context-window-aware, circuit breaker | Remote/local split |
