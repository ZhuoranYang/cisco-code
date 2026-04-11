# Claw-Code-Parity — Architecture Analysis & Claude Code Proximity

Source: `/Users/zhuoran_cisco/Documents/agent_tools/tmp/claw-code-parity`
Language: Rust (primary), Python (parity audit scaffold)
Workspace: `rust/` with 8 crates: `api`, `commands`, `compat-harness`, `plugins`, `runtime`, `rusty-claude-cli`, `telemetry`, `tools`

---

## 1. Knowledge Management

### CLAUDE.md Handling (`rust/crates/runtime/src/prompt.rs`)

`discover_instruction_files()` (line 192) walks ancestor chain from cwd to root, checking per directory:
- `CLAUDE.md`
- `CLAUDE.local.md`
- `.claw/CLAUDE.md`
- `.claw/instructions.md`

Files deduplicated by content hash (`stable_content_hash`).
Truncated: 4,000 chars per file (`MAX_INSTRUCTION_FILE_CHARS`), 12,000 chars total.
Injected under `# Claude instructions` section via `render_instruction_files`.

### Memory System

**No persistent memory store.** Only session JSONL serves as "memory".
No `SessionMemory`, no team memory sync, no scratchpad.
PARITY.md explicitly calls scratchpad out as missing.

---

## 2. Session Management (`rust/crates/runtime/src/session.rs`)

- JSONL format stored at `.claude/sessions/session-{timestamp}-{counter}.json`
- `generate_session_id()` uses `{millis}-{atomic_counter}`
- Atomic writes via temp file + rename
- Incremental append via `append_persisted_message()`
- Rotation at `ROTATE_AFTER_BYTES = 256 * 1024`, keeps `MAX_ROTATED_FILES = 3`
- **Session fork**: `Session::fork()` clones history with `SessionFork { parent_session_id, branch_name }`
- **Compaction metadata** stored inline in JSONL as `compaction` record type

---

## 3. Context Management (`rust/crates/runtime/src/prompt.rs`)

### System Prompt Assembly

`SystemPromptBuilder::build()` (line 134):

1. Intro section
2. Output style (optional)
3. `# System` section
4. `# Doing tasks` section
5. `# Executing actions with care`
6. `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` marker
7. `# Environment context` (cwd, date, OS, model family)
8. `# Project context` (git status/diff snapshot)
9. `# Claude instructions` (CLAUDE.md files)
10. `# Runtime config` (loaded settings)

### Token Estimation (`rust/crates/runtime/src/compact.rs`)

`estimate_session_tokens()`: rough heuristic `text.len() / 4 + 1` per block.

### Compaction

`compact_session()` (line 89):
- Triggered via `should_compact()` (configurable `CompactionConfig`)
- Default: 4 preserved recent messages, 10,000 estimated token threshold
- Summarizes removed messages into `<summary>` XML block
- Replaces with single `System` message + `COMPACT_CONTINUATION_PREAMBLE`
- Merges with prior compaction summary via `merge_compact_summaries`

---

## 4. Skills / Plugins

### Tool System (`rust/crates/tools/src/lib.rs`)

`GlobalToolRegistry` backed by `mvp_tool_specs()`:
`bash`, `read_file`, `write_file`, `edit_file`, `glob_search`, `grep_search`,
`WebFetch`, `WebSearch`, `TodoWrite`, `Skill`, `Agent`, `ToolSearch`,
`NotebookEdit`, `Sleep`, `SendUserMessage`, `Config`, `StructuredOutput`, `REPL`, `PowerShell`

### Skills

`Skill` tool loads local `SKILL.md` files directly.
No bundled skill registry, no `/skills` slash command, no `loadSkillsDir` equivalent.

### Plugin System (`rust/crates/plugins/src/lib.rs`)

Exists (added after PARITY.md was written):
- `PluginKind`: Builtin/Bundled/External
- `PluginMetadata`, `PluginHooks` (PreToolUse/PostToolUse/PostToolUseFailure)
- `PluginTool` injectable into `GlobalToolRegistry`
- Bundled plugins under `rust/crates/plugins/bundled/` with shell hook scripts
- Hook execution in `hooks.rs`

### MCP Integration

- Config in `runtime/src/config.rs`: supports `stdio`, `sse`, `http`, `ws`, `sdk`, `claudeai-proxy` transports
- Tool name generation in `runtime/src/mcp.rs`
- Full MCP stdio client in `mcp_stdio.rs` and `mcp_client.rs`

---

## 5. Config (`rust/crates/runtime/src/config.rs`)

Hierarchy (later wins):
1. `~/.claw.json` (legacy user compat)
2. `$CLAW_CONFIG_HOME/settings.json` (user, defaults to `~/.claw/`)
3. `$cwd/.claw.json` (project compat)
4. `$cwd/.claw/settings.json` (project)
5. `$cwd/.claw/settings.local.json` (local override)

Key fields: `model`, `permissionMode`, `permissions.{allow,deny,ask,defaultMode}`,
`hooks.{PreToolUse,PostToolUse,PostToolUseFailure}`, `mcpServers`, `plugins`,
`enabledPlugins`, `sandbox`, `oauth`.

Deep-merging via `deep_merge_objects`. Env override: `CLAW_CONFIG_HOME`.

---

## 6. How Close Is Claw-Code-Parity to Real Claude Code?

### Verdict: **Clean-room Rust reimplementation — ~40-50% feature coverage**

### What It IS:
- A deliberate Rust port by someone with deep knowledge of Claude Code's external behavior
- Written from scratch after studying the extracted TypeScript source
- NOT decompiled or extracted code — idiomatic Rust with fresh variable names and organization
- The PARITY.md shows systematic gap analysis against upstream TypeScript

### Architecture Similarity (HIGH):
- Config vocabulary mirrors Claude Code: `settings.json`, permission modes (`acceptEdits`, `dontAsk`)
- `__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__` marker and section order replicate the real structure
- CLAUDE.md discovery, session JSONL format, compaction via system-role summary all match
- MCP transport types, hook lifecycle names, OAuth config shape all mirror public API surface
- `.claw/` directory structure is a 1:1 rename of `.claude/`

### Coverage Gaps (SIGNIFICANT):
| Area | Status |
|------|--------|
| Core runtime/API/session/config | Strong (~80%) |
| CLAUDE.md loading | Good (missing rules/*.md, @include) |
| Memory system | Missing entirely |
| Scratchpad | Missing |
| Skills loading from dirs | Missing (only inline SKILL.md) |
| Bundled skills | Missing |
| Hook execution pipeline | Partial |
| Plugin install/manage surface | Basic |
| CLI breadth | Missing many commands |
| Streaming tool orchestration | Partial |
| Analytics/LSP/team memory | Missing |
| Prompt suggestions | Missing |

### Compared to Extracted Claude Code TypeScript:

The extracted TypeScript source at `explore_claude_code/extracted_src_v2.1.88` IS the real Claude Code.
Claw-code-parity is a **reverse-engineered reimplementation** that captures the architecture
but not the full feature set. The TypeScript is canonical — it has the complete skill system,
memory pipeline, hook execution, permission rules, conditional skills, team memory,
compaction circuit breakers, and all the feature gates.

### Which is closer to Claude Code source?

**The extracted TypeScript IS Claude Code source code.** It contains the actual implementation
with all internal feature flags, Statsig gates, ant-only code paths, and the full skill/plugin/hook
pipeline. Claw-code-parity is a well-crafted approximation that captures ~40-50% of the
functionality in a different language.
