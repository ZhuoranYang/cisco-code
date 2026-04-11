# 05 — Competitive Parity Build Plan

> Staged plan to bring cisco-code to feature parity with Claude Code and surpass Codex
> across agent loop, tools, context management, prompt assembly, and commands.

---

## Gap Analysis Summary

### Current State (cisco-code)
- 32 built-in tools, 10+ providers, 37 slash commands, 10 bundled skills
- Sequential tool execution in a ReAct loop
- Single-level full compaction (basic 5-point summarization prompt)
- Layered prompt with cache boundary (static/dynamic split)
- 9 hook events, MPSC event channel, 10 StreamEvent variants
- ToolProgress defined in protocol but never emitted

### Gaps vs Claude Code

| Area | Gap | Impact |
|------|-----|--------|
| Agent loop | No streaming tool concurrency | Tools block streaming; 2-5x slower on multi-tool turns |
| Agent loop | No denial tracking | Agent retries denied tools in infinite loops |
| Agent loop | No auto-background for long tasks | User waits for slow bash commands |
| Tools | No `apply_patch` tool | Model must use Edit for multi-hunk changes |
| Tools | No `TodoWrite` tool | No structured todo tracking |
| Tools | Bash missing `run_in_background` | Can't offload long commands |
| Tools | Agent missing worktree isolation | Sub-agents conflict on filesystem |
| Tools | Grep missing multiline, offset/head_limit | Can't search across lines or paginate results |
| Tools | Read missing PDF/image support | Can't handle binary files |
| Compaction | No partial/microcompact | All-or-nothing; loses nuance |
| Compaction | Basic 5-point prompt | Claude Code has 9-section detailed prompt |
| Compaction | No file snapshots | Can't restore file state post-compaction |
| Compaction | No post-compact restoration | Loses skill/file context after compaction |
| Compaction | No function result clearing | Old tool results bloat context |
| Prompt | No system-reminder injection | Can't inject contextual guidance mid-conversation |
| Prompt | No dynamic section registry | No per-section memoization/invalidation |
| Prompt | Git context doesn't detect changes | `has_uncommitted` always false |
| Prompt | No MCP instructions section | Connected MCP servers can't add guidance |
| Prompt | No prompt cache control metadata | Can't leverage API-level caching |
| Prompt | No task/todo context in prompt | Model unaware of tracked tasks |
| Commands | No skill forked execution | Skills share main context (risk of pollution) |
| Commands | No skill model override enforcement | Can't run cheap skills on Haiku |
| Hooks | Missing events | No FileChanged, CompactionStart/End, UserPromptSubmit |
| Hooks | No hook-based permission decisions | Hooks can suppress but not approve |

### Gaps vs Codex (where Codex is ahead)

| Area | Gap | Impact |
|------|-----|--------|
| Tools | No apply_patch with grammar validation | Less robust multi-file edits |
| Tools | No JS REPL | Can't run JS interactively |
| Context | No token-budget-aware truncation | Crude estimation, no per-section budgets |
| IDE | No app-server JSON-RPC protocol | No IDE extension support |
| Sandbox | No network proxy enforcement | Network policy is config-only |
| Exec policy | Hardcoded regex patterns | Not user-extensible |

---

## Staged Build Plan

### Stage 1: Agent Loop & Streaming Foundation
**Goal**: Match Claude Code's streaming tool execution and loop robustness
**Estimated scope**: 5 files changed, ~800 LOC

#### 1.1 — Streaming Tool Concurrency

**Files**: `crates/protocol/src/tools.rs`, `crates/tools/src/lib.rs`, `crates/runtime/src/conversation.rs`

Add `is_concurrency_safe` flag to tool trait and implement parallel execution:

```rust
// In Tool trait (crates/tools/src/lib.rs)
pub trait Tool: Send + Sync {
    // ... existing methods ...
    fn is_concurrency_safe(&self) -> bool { false } // default: sequential
}

// Concurrency-safe tools (read-only, no side effects):
// - Read, Grep, Glob, LSP, ToolSearch, TaskGet, TaskList, TaskOutput,
//   CronList, ListMcpResources, ReadMcpResource, Sleep
//
// NOT safe (side effects, filesystem writes, external calls):
// - Bash, Write, Edit, Agent, WebFetch, WebSearch, NotebookEdit,
//   Skill, EnterPlanMode, ExitPlanMode, TaskCreate, TaskUpdate,
//   TaskStop, CronCreate, CronDelete, EnterWorktree, ExitWorktree,
//   AskUserQuestion, SendMessage, Config
```

Build a `StreamingToolExecutor` in `crates/runtime/src/streaming_executor.rs`:

```
Queue tools as model streams them in.
When tool arrives:
  if queue is empty → execute immediately
  if tool is_concurrency_safe AND all executing tools are safe → execute in parallel
  else → wait for current tools to finish, then execute exclusively
Collect results in call order (not completion order).
```

**In conversation.rs** (lines 488-638), replace the sequential `for` loop with:

```rust
let executor = StreamingToolExecutor::new(&self.tools, &self.permissions);
let results = executor.execute_all(tool_uses, &tool_ctx).await;
for result in results {
    // emit events, add to session (same as current code)
}
```

#### 1.2 — ToolProgress Emission

**Files**: `crates/tools/src/bash.rs`, `crates/tools/src/grep.rs`, `crates/tools/src/web_fetch.rs`

Wire up the existing `ToolProgress` event variants:

- **Bash**: Emit `ToolProgress::Command { stdout_line }` per line of output
- **Grep**: Emit `ToolProgress::Search { files_searched, matches_found }` periodically
- **WebFetch**: Emit `ToolProgress::Generic { message: "Fetching...", percentage }` on download progress

Add a `progress_tx: Option<mpsc::Sender<StreamEvent>>` to `ToolContext` so tools can emit progress.

#### 1.3 — Denial Tracking & Loop Prevention

**Files**: `crates/runtime/src/permissions.rs`, `crates/runtime/src/conversation.rs`

```rust
// In PermissionEngine
struct DenialTracker {
    denials: HashMap<String, Vec<Instant>>,  // tool_name → timestamps
    max_denials: usize,                       // default: 3
    window: Duration,                         // default: 60s
}

impl DenialTracker {
    fn record_denial(&mut self, tool_name: &str);
    fn should_block(&self, tool_name: &str) -> bool;
    // Returns true if tool has been denied >= max_denials times within window
}
```

In the agent loop, after permission denial:
- Record denial in tracker
- If `should_block()`, inject a system message telling the model to stop attempting this tool
- Emit `Error { message: "Tool {name} denied {n} times, stopping attempts", recoverable: true }`

#### 1.4 — Auto-Background for Long Tasks

**Files**: `crates/tools/src/bash.rs`, `crates/protocol/src/tools.rs`

Add `run_in_background: Option<bool>` to Bash tool input schema.

When `run_in_background = true`:
- Spawn command via `tokio::spawn`
- Return immediately with `{ "background_task_id": "<uuid>", "status": "launched" }`
- Write stdout/stderr to `~/.cisco-code/tasks/<id>.txt`
- TaskOutput tool can poll the file

When bash command exceeds 120s (configurable):
- Auto-background: move to background task
- Notify user via StreamEvent
- Return partial output + task ID

---

### Stage 2: Tool Coverage
**Goal**: Match Claude Code's tool set (excluding Claude-specific tools)
**Estimated scope**: 4 new files, 3 files enhanced, ~1200 LOC

#### 2.1 — ApplyPatch Tool

**New file**: `crates/tools/src/apply_patch.rs`

Unified diff application tool (inspired by Codex's grammar-validated approach):

```rust
pub struct ApplyPatchTool;

// Input schema:
// - patch: String (unified diff format)
// - cwd: Option<String> (working directory, default: session cwd)
//
// Validation:
// 1. Parse unified diff headers (---/+++ lines)
// 2. Validate hunk headers (@@ -start,count +start,count @@)
// 3. Verify context lines match file content
// 4. Apply hunks in order
// 5. Return list of modified files + success/failure per hunk
//
// Permission: WorkspaceWrite (same as Edit)
// Error handling: If any hunk fails, report which ones succeeded and which failed
```

Why this matters: Models naturally produce unified diffs for multi-file changes. Forcing them through Edit (one replacement at a time) wastes tokens and is error-prone.

#### 2.2 — TodoWrite Tool

**New file**: `crates/tools/src/todo_tool.rs`

Structured todo management (persisted to `.cisco-code/todos.json`):

```rust
pub struct TodoWriteTool;

// Input schema:
// - todos: Vec<TodoItem>
//   where TodoItem = { id: String, content: String, status: "pending"|"in_progress"|"done", priority: Option<u8> }
//
// Replaces entire todo list (write-mode, not append)
// Persisted to .cisco-code/todos.json in project root
// Read by prompt builder → injected into dynamic section
//
// Permission: ReadOnly (metadata only, no filesystem side effects beyond project config)
```

#### 2.3 — Bash Enhancements

**File**: `crates/tools/src/bash.rs`

Add to input schema:
- `run_in_background: Option<bool>` — launch as background task (from Stage 1.4)
- `description: Option<String>` — human-readable operation description (shown in UI)

Add to behavior:
- Background task file writing to `~/.cisco-code/tasks/<id>.txt`
- Integration with TaskOutput for polling

#### 2.4 — Grep Enhancements

**File**: `crates/tools/src/grep.rs`

Add to input schema:
- `multiline: Option<bool>` — enable cross-line matching (`rg -U --multiline-dotall`)
- `head_limit: Option<u32>` — limit output to first N results (default: 250)
- `offset: Option<u32>` — skip first N results before applying head_limit
- `-i: Option<bool>` — case-insensitive search
- `type: Option<String>` — file type filter (`rg --type`)

These match Claude Code's Grep tool schema exactly.

#### 2.5 — Read Enhancements

**File**: `crates/tools/src/read.rs`

Add:
- PDF support: detect `.pdf` extension, use `pdf-extract` crate or shell out to `pdftotext`
  - Add `pages: Option<String>` parameter (e.g., "1-5", "3", "10-20")
  - Max 20 pages per request
- Image support: detect image extensions, return base64-encoded content
  - The model receives the image as a content block

#### 2.6 — Agent Tool Enhancements

**File**: `crates/tools/src/agent.rs`

Add to input schema:
- `isolation: Option<String>` — `"worktree"` creates git worktree for agent
- `model: Option<String>` — override model for sub-agent (e.g., "sonnet", "opus", "haiku")
- `run_in_background: Option<bool>` — launch agent as background task

Worktree isolation flow:
1. `git worktree add .cisco-code/worktrees/<uuid> -b agent/<uuid>`
2. Set agent's `cwd` to worktree path
3. On completion: if changes made, return worktree path + branch name
4. Cleanup worktrees with no changes

---

### Stage 3: Context Management
**Goal**: Match Claude Code's multi-level compaction with restoration
**Estimated scope**: 2 files rewritten, 1 new file, ~600 LOC

#### 3.1 — Enhanced Compaction Prompt

**File**: `crates/runtime/src/compact.rs` (rewrite lines 197-207)

Replace the basic 5-point prompt with Claude Code's 9-section structure:

```
You are a conversation summarizer. Create a detailed summary preserving:

1. PRIMARY REQUEST — The user's main goal and intent
2. KEY TECHNICAL CONCEPTS — Frameworks, patterns, constraints discussed
3. FILES AND CODE — Every file path mentioned, with full code snippets for
   functions/classes that were created or modified. Include line numbers.
4. ERRORS AND FIXES — Error messages encountered and how they were resolved
5. PROBLEM-SOLVING CONTEXT — Approaches tried, what worked, what didn't
6. ALL USER MESSAGES — Every distinct request from the user (not tool results)
7. PENDING TASKS — Any incomplete work or next steps mentioned
8. CURRENT WORK STATE — What was being worked on when this summary was created,
   including exact file names and current progress
9. NEXT STEP — The most likely next action, with quotes from recent context

IMPORTANT: Preserve ALL file paths, function names, and code snippets verbatim.
The summary replaces the original messages — lost details cannot be recovered.

Output your analysis in <analysis> tags (will be stripped), then the summary.
```

Increase `summary_max_tokens` from 2048 to 8192 (Claude Code uses 50K budget).

#### 3.2 — Partial Compaction (Microcompact)

**New file**: `crates/runtime/src/microcompact.rs`

Lightweight compaction for when full compaction is too aggressive:

```rust
pub struct MicroCompactor {
    // Configuration
    max_tool_results_to_keep: usize,  // default: 5
    max_chars_per_result: usize,      // default: 2000
}

impl MicroCompactor {
    /// Clears old tool results, keeping only the most recent N.
    /// Replaces cleared results with "[result cleared — see summary above]"
    pub fn clear_old_results(&self, messages: &mut Vec<Message>) -> usize;

    /// Truncates large tool results to max_chars_per_result.
    pub fn truncate_results(&self, messages: &mut Vec<Message>) -> usize;
}
```

Integration into agent loop (after each turn):
1. If `estimated_tokens > 50% of threshold`: run microcompact (clear old results)
2. If `estimated_tokens > 80% of threshold`: run full compaction
3. If `estimated_tokens > 90% of threshold`: emergency compaction with reduced `preserve_recent`

#### 3.3 — Post-Compaction File Restoration

**File**: `crates/runtime/src/compact.rs` (add to `compact()` method)

After full compaction:
1. Identify the 5 most recently modified files from compacted messages
2. Re-read each file (up to 5K tokens each, 50K total budget)
3. Inject as a system message: `"[Post-compaction file snapshot]\n\nFile: {path}\n```\n{content}\n```"`
4. Re-inject most recently used skill content (up to 25K token budget)

```rust
struct PostCompactRestoration {
    max_files: usize,           // 5
    max_tokens_per_file: usize, // 5000
    total_token_budget: usize,  // 50000
    max_skills: usize,          // 3
    skill_token_budget: usize,  // 25000
}
```

#### 3.4 — Function Result Clearing (FRC)

**File**: `crates/runtime/src/compact.rs` or `microcompact.rs`

Between compaction rounds, aggressively clear old tool result content:
- Keep the last N tool results intact (configurable, default 3)
- Replace older tool result content with `"[cleared]"`
- Preserve tool name and call ID for reference

This is Claude Code's "summarize_tool_results" / FRC mechanism — it prevents
context bloat from accumulated grep/read results.

---

### Stage 4: Prompt Assembler
**Goal**: Match Claude Code's layered, cache-optimized prompt with dynamic sections
**Estimated scope**: 1 file rewritten, 2 new files, ~500 LOC

#### 4.1 — Dynamic Section Registry

**New file**: `crates/runtime/src/prompt_sections.rs`

Replace the monolithic `PromptBuilder` with a section registry:

```rust
pub struct PromptSection {
    pub name: &'static str,
    pub content: String,
    pub cacheable: bool,       // true = goes before cache boundary
    pub memoized: bool,        // true = computed once, reused until invalidated
}

pub struct PromptSectionRegistry {
    sections: Vec<PromptSection>,
    cache: HashMap<String, String>,  // memoized section cache
}

impl PromptSectionRegistry {
    pub fn register_cached(&mut self, name: &str, content: String);
    pub fn register_dynamic(&mut self, name: &str, compute: impl Fn() -> String);
    pub fn invalidate(&mut self, name: &str);
    pub fn build(&self) -> (String, String); // (static_prefix, dynamic_suffix)
}
```

**Sections (in order)**:

Static (cacheable):
1. `core` — Agent identity and behavior
2. `system` — Tool execution rules, system-reminder guidance
3. `doing_tasks` — Task approach guidelines
4. `actions` — Reversibility assessment, confirmation rules
5. `using_tools` — Dedicated tool vs Bash preference
6. `tone_style` — Conciseness, formatting
7. `output_efficiency` — Brief, direct output guidance
8. `tool_guidelines` — Per-tool usage tips

Dynamic (per-turn):
9. `session_guidance` — Skills, agent guidance (memoized, invalidate on /clear)
10. `environment` — CWD, platform, shell, model, git status
11. `memory` — MEMORY.md content (memoized, invalidate on memory write)
12. `mcp_instructions` — From connected MCP servers (uncached)
13. `todos` — Current todo list from TodoWrite (uncached)
14. `scratchpad` — Scratchpad directory (memoized)
15. `date` — Current date (uncached)

#### 4.2 — System-Reminder Injection

**File**: `crates/runtime/src/conversation.rs`

Add mechanism to inject `<system-reminder>` tags into user/tool messages:

```rust
impl ConversationRuntime<P> {
    /// Inject a system reminder that will appear in the next turn's context.
    pub fn inject_system_reminder(&mut self, content: &str) {
        self.pending_reminders.push(content.to_string());
    }
}
```

In `build_api_messages()`, append pending reminders to the last user message:

```rust
if !self.pending_reminders.is_empty() {
    let reminders = self.pending_reminders.drain(..)
        .map(|r| format!("<system-reminder>\n{r}\n</system-reminder>"))
        .collect::<Vec<_>>()
        .join("\n");
    // Append to last user message content
}
```

Use cases:
- Task reminders ("You have N pending tasks")
- Skill availability notifications
- Hook-injected context
- MCP server status changes

#### 4.3 — Enhanced Git Context

**File**: `crates/runtime/src/prompt.rs` (replace `detect_git_context()`)

Actually run `git status` and `git log` to get real info:

```rust
pub async fn detect_git_context(cwd: &str) -> GitContext {
    // 1. Check .git exists
    // 2. Run: git rev-parse --abbrev-ref HEAD → branch
    // 3. Run: git status --porcelain → uncommitted changes (truncate at 2KB)
    // 4. Run: git log --oneline -5 → recent commits
    // 5. Run: git config user.name → user name
    GitContext {
        is_repo: true,
        branch: Some("main"),
        has_uncommitted: true,
        status_summary: "3 files modified, 1 untracked",
        recent_commits: vec!["abc1234 fix: auth bug", ...],
        user_name: Some("zhuoran"),
    }
}
```

#### 4.4 — MCP Instructions Section

**File**: `crates/runtime/src/prompt.rs`

When MCP servers are connected, collect their instructions:

```rust
fn build_mcp_instructions(&self) -> Option<String> {
    let mut sections = Vec::new();
    for (server_name, client) in &self.mcp_clients {
        if let Some(instructions) = client.server_instructions() {
            sections.push(format!("## {server_name}\n{instructions}"));
        }
    }
    if sections.is_empty() { None }
    else { Some(sections.join("\n\n")) }
}
```

Inject as dynamic (uncached) section — MCP servers may connect/disconnect between turns.

#### 4.5 — Prompt Cache Control Metadata

**File**: `crates/api/src/client.rs`

When building the API request body, split system prompt at cache boundary:

```rust
// Static prefix → cache_control: { type: "ephemeral" } (or "global" for Anthropic)
// Dynamic suffix → no cache control
let (static_prefix, dynamic_suffix) = prompt_registry.build();
let system_blocks = vec![
    SystemBlock { text: static_prefix, cache_control: Some("ephemeral") },
    SystemBlock { text: dynamic_suffix, cache_control: None },
];
```

This leverages Anthropic's prompt caching to avoid re-tokenizing the static portion.

---

### Stage 5: Commands, Hooks & Polish
**Goal**: Full skill system parity and hook coverage
**Estimated scope**: 3 files enhanced, ~400 LOC

#### 5.1 — Skill Forked Execution

**File**: `crates/tools/src/skill.rs`, `crates/runtime/src/conversation.rs`

Add `context` field to skill YAML frontmatter:

```yaml
---
name: code-review
context: fork  # or "inline" (default)
model: sonnet  # optional model override
allowed-tools: [Read, Grep, Glob, Bash]
---
```

When `context: fork`:
1. Create a new `ConversationRuntime` with the skill's system prompt
2. Use the skill's `model` override (or inherit parent)
3. Restrict tools to `allowed-tools` list
4. Execute skill prompt in isolated context
5. Return result as tool output (not polluting main context)

When `context: inline` (default):
- Current behavior — expand skill content into main conversation

#### 5.2 — Additional Hook Events

**File**: `crates/runtime/src/hooks.rs`

Add missing events to match Claude Code:

```rust
pub enum HookEvent {
    // ... existing 9 events ...
    UserPromptSubmit { prompt: String },      // Before processing user input
    FileChanged { path: String, op: String }, // After file write/edit
    CompactionStart,                          // Before compaction
    CompactionEnd { summary_tokens: u64 },    // After compaction
    Setup,                                    // One-time setup on first session
}
```

Wire into agent loop:
- `UserPromptSubmit`: Fire at start of `run_agent_loop()`, before adding message
- `FileChanged`: Fire in Write/Edit tool post-execution
- `CompactionStart/End`: Fire around compaction in the loop
- `Setup`: Fire on first session creation

#### 5.3 — Hook-Based Permission Decisions

**File**: `crates/runtime/src/hooks.rs`, `crates/runtime/src/conversation.rs`

Enhance `PreToolUse` hook result to support explicit permission decisions:

```rust
pub enum HookResult {
    Continue,                    // Proceed normally
    ContinueWithModifiedInput(Value), // Proceed with modified tool input
    Suppress,                    // Skip tool execution
    Approve,                     // NEW: Override permission → auto-approve
    Deny { reason: String },     // NEW: Override permission → deny
    Error(String),               // Hook failed
}
```

In the agent loop, check hook result BEFORE permission engine:
- `Approve` → skip permission check, execute tool
- `Deny` → skip permission check, return error

This enables enterprise policy hooks that programmatically control tool access.

#### 5.4 — Todo Context in Prompt

**File**: `crates/runtime/src/prompt.rs`

Add todo injection to dynamic sections:

```rust
fn load_todo_context(cwd: &str) -> Option<String> {
    let path = Path::new(cwd).join(".cisco-code/todos.json");
    if !path.exists() { return None; }
    let todos: Vec<TodoItem> = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    if todos.is_empty() { return None; }
    let text = todos.iter()
        .map(|t| format!("- [{}] {}", if t.status == "done" { "x" } else { " " }, t.content))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!("# Current Todos\n\n{text}"))
}
```

---

## Stage Dependencies

```
Stage 1 (Agent Loop)  ──→  Stage 2 (Tools)  ──→  Stage 5 (Polish)
                                ↓
                       Stage 3 (Context)  ──→  Stage 4 (Prompt)
```

- Stage 1 is prerequisite: streaming executor needed before tool enhancements
- Stages 2 and 3 can run in parallel after Stage 1
- Stage 4 depends on Stage 3 (compaction changes affect prompt sections)
- Stage 5 depends on Stages 2 and 4

## Wave Execution Plan (for /astro-loop)

**Wave 1** (parallel): Stage 1.1 + 1.2 + 1.3 + 1.4
**Wave 2** (parallel): Stage 2.1 + 2.2 + 2.4 + 2.5 (independent tool files)
**Wave 3** (parallel): Stage 2.3 + 2.6 + Stage 3.1 + 3.2
**Wave 4** (parallel): Stage 3.3 + 3.4 + Stage 4.1
**Wave 5** (parallel): Stage 4.2 + 4.3 + 4.4 + 4.5
**Wave 6** (parallel): Stage 5.1 + 5.2 + 5.3 + 5.4

## Testing Strategy

Each stage must:
1. Add unit tests for new structs/functions
2. Add integration tests for cross-component flows
3. Pass existing test suite (no regressions)
4. Pass `cargo clippy` and `cargo test`

Key test scenarios:
- **Stage 1**: Test concurrent tool execution with mock tools; test denial after N denials
- **Stage 2**: Test apply_patch with various diff formats; test background bash polling
- **Stage 3**: Test microcompact preserves recent results; test file restoration limits
- **Stage 4**: Test prompt section ordering; test cache boundary stability
- **Stage 5**: Test forked skill isolation; test hook permission override

## Success Criteria

After all stages:
- [x] Streaming tool concurrency: concurrent-safe tools execute in parallel
- [x] All Claude Code tools present (minus Claude-specific: Brief, REPL, PowerShell, WebBrowser)
- [x] Multi-level compaction: micro (50%), full (80%), emergency (90%)
- [x] Post-compaction file/skill restoration
- [x] System-reminder injection working
- [x] Dynamic section registry with memoization
- [x] Git context shows real uncommitted changes
- [x] Skill forked execution isolates context
- [x] 13+ hook events (up from 9)
- [x] Denial tracking prevents infinite loops
- [x] Background bash tasks with polling
- [x] Todo tracking in prompt context
