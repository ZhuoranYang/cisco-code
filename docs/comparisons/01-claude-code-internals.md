# Claude Code v2.1.88 — Internal Architecture Analysis

Source: `/Users/zhuoran_cisco/Documents/agent_tools/explore_claude_code/extracted_src_v2.1.88`
Language: TypeScript (Node.js)

---

## 1. Knowledge Management

### CLAUDE.md Loading (`src/utils/claudemd.ts`)

The canonical loader is `getMemoryFiles()` (memoized). Load order (lowest to highest priority):

1. **Managed** — `/etc/claude-code/CLAUDE.md` (enterprise policy)
2. **Managed rules** — `<managed_path>/.claude/rules/*.md`
3. **User** — `~/.claude/CLAUDE.md`
4. **User rules** — `~/.claude/rules/*.md`
5. **Project walk** — traverses from filesystem root down to CWD, checking per directory:
   - `<dir>/CLAUDE.md` (type: `Project`)
   - `<dir>/.claude/CLAUDE.md` (type: `Project`)
   - `<dir>/.claude/rules/*.md` (type: `Project`)
   - `<dir>/CLAUDE.local.md` (type: `Local`)
6. **Additional dirs** — if `CLAUDE_CODE_ADDITIONAL_DIRECTORIES_CLAUDE_MD=1`, repeats for `--add-dir` paths
7. **AutoMem** — `~/.claude/projects/<sanitized-cwd>/memory/MEMORY.md`
8. **TeamMem** — team memory entrypoint (feature-gated `TEAMMEM`)

Walk processes from root down to CWD so closer directories override further ones.
Max `@include` depth is 5, circular refs tracked in a `Set<string>`.
`claudeMdExcludes` in settings can suppress specific paths via picomatch.

### Auto-Memory / MEMORY.md (`src/memdir/paths.ts`)

`getAutoMemPath()` computes:
```
<memoryBase>/projects/<sanitizePath(canonicalGitRoot)>/memory/
```
Where `memoryBase` defaults to `~/.claude` (overridable via `CLAUDE_CODE_REMOTE_MEMORY_DIR`).

Entry point: `MEMORY.md` in that directory.
Daily log files: `<autoMemPath>/logs/YYYY/MM/YYYY-MM-DD.md` (feature `KAIROS`).

`isAutoMemoryEnabled()` checks (in order):
- `CLAUDE_CODE_DISABLE_AUTO_MEMORY` env
- `CLAUDE_CODE_SIMPLE` (bare mode)
- CCR without memory dir
- `autoMemoryEnabled` in settings

Memory types (`src/memdir/memoryTypes.ts`): `user`, `feedback`, `project`, `reference`.
Each file uses YAML frontmatter with `name`, `description`, `type` fields.

### Scratchpad (`src/utils/permissions/filesystem.ts`, `src/constants/prompts.ts`)

`isScratchpadEnabled()` checks Statsig gate `tengu_scratch`.
`getScratchpadDir()` returns per-session temp dir under `/tmp/claude-<uid>/`.

When enabled, a `'scratchpad'` section is injected into the system prompt:
```typescript
systemPromptSection('scratchpad', () => getScratchpadInstructions())
```
Instruction tells the model to use this directory instead of `/tmp`.

### Agents/Custom Agents (`src/tools/AgentTool/loadAgentsDir.ts`)

Agent definitions live in `.claude/agents/` directories.
Format: markdown with frontmatter specifying `name`, `description`, `allowedTools`, `permissionMode`, etc.
Built-in agents: `verificationAgent`, `planAgent`, `exploreAgent`, `claudeCodeGuideAgent`.

---

## 2. Session Management

### Storage Format (`src/utils/sessionStorage.ts`)

Sessions stored as JSONL files:
```
~/.claude/projects/<sanitizePath(originalCwd)>/<sessionId>.jsonl
```

Sub-agent transcripts:
```
<projectDir>/<sessionId>/subagents/agent-<agentId>.jsonl
```

JSONL entry types: `user`, `assistant`, `attachment`, `system`, plus non-persisted `progress` entries.
A `SystemCompactBoundaryMessage` marks compaction points in the chain.

### Session Resume (`src/utils/sessionRestore.ts`)

`switchSession()` sets the active session.
Resume re-reads JSONL via `readTranscriptForLoad()`, reconstructs:
- `fileHistorySnapshots`
- `attributionSnapshots`
- `contextCollapseCommits`
- Todo list from last `TodoWriteTool` call

### Session Listing

`getProjectsDir()` is scanned; for each project directory, `.jsonl` files enumerated.
First-prompt extraction uses `SKIP_FIRST_PROMPT_PATTERN` to skip synthetic messages.
Session metadata (name, last used, tokens, cost) appended via `reAppendSessionMetadata`.

---

## 3. Context Management

### System Prompt Assembly (`src/constants/prompts.ts`)

`getSystemPrompt()` assembles sections via `systemPromptSection()` / `resolveSystemPromptSections()`:

| # | Section | Description |
|---|---------|-------------|
| 1 | intro | Identity and capabilities |
| 2 | session_guidance | Task reminders |
| 3 | memory | CLAUDE.md + MEMORY.md via `loadMemoryPrompt()` |
| 4 | ant_model_override | Internal-only suffix |
| 5 | env_info_simple | OS, cwd, git status, date |
| 6 | language | Language preference |
| 7 | output_style | Output formatting |
| 8 | **DYNAMIC_BOUNDARY** | **Cache split point** |
| 9 | scratchpad | Temp dir instructions |
| 10 | frc | Function result clearing |
| 11 | MCP/skills/tools | Dynamic tool sections |
| 12 | brief | (KAIROS feature) |

### Cache Boundaries (`src/constants/prompts.ts`, line 114)

```typescript
export const SYSTEM_PROMPT_DYNAMIC_BOUNDARY = '__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__'
```

Everything before = `scope: 'global'` (cross-org prompt cache).
Everything after = session-specific.
Controlled by `shouldUseGlobalCacheScope()`.

### Token Counting (`src/utils/context.ts`, `src/utils/tokens.ts`)

- `getContextWindowForModel()`: 200K default, 1M for `[1m]`-suffix models
- `tokenCountWithEstimation()`: uses actual API usage when available, falls back to estimation
- Override: `CLAUDE_CODE_MAX_CONTEXT_TOKENS` env (ant-only)

### Auto-Compaction (`src/services/compact/autoCompact.ts`)

Threshold: `getAutoCompactThreshold(model)` = `effectiveContextWindow - 13_000`
Where `effectiveContextWindow` = context_window - `min(maxOutputTokens, 20_000)`.

Override: `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` (percentage).
Disable: `DISABLE_COMPACT`, `DISABLE_AUTO_COMPACT`, `autoCompactEnabled: false`.

Flow:
1. `autoCompactIfNeeded()` — main entry, called from query loop
2. First tries `trySessionMemoryCompaction()` (session-memory pruning)
3. Falls back to full `compactConversation()`
4. Circuit breaker after `MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES = 3`
5. Writes `SystemCompactBoundaryMessage` to JSONL

---

## 4. Skills / Plugins

### Built-in Skills (`src/skills/bundled/index.ts`)

`initBundledSkills()` registers:
- `update-config`, `keybindings`, `verify`, `debug`, `lorem-ipsum`, `skillify`, `remember`, `simplify`, `batch`, `stuck`
- Feature-gated: `dream` (KAIROS), `hunter` (REVIEW_ARTIFACT), `loop`, `schedule-remote-agents`, `claude-api`, `claude-in-chrome`, `run-skill-generator`

### Custom Skill Loading (`src/skills/loadSkillsDir.ts`)

`getSkillDirCommands()` loads from (priority order):
1. `<managed_path>/.claude/skills/`
2. `~/.claude/skills/`
3. Project dirs CWD→$HOME: `<dir>/.claude/skills/`
4. `--add-dir` paths
5. Legacy `.claude/commands/`

Format: `skill-name/SKILL.md` with YAML frontmatter fields:
- `name`, `description`, `when_to_use`, `allowed-tools`, `argument-hint`
- `arguments`, `version`, `model`, `disable-model-invocation`
- `user-invocable`, `hooks`, `context` (fork/inline), `agent`, `effort`
- `shell`, `paths` (conditional activation globs)

### Conditional Skills

Skills with `paths:` frontmatter stored in `conditionalSkills` map.
Activated when matching file is touched (`activateConditionalSkillsForPaths()`).
New skill dirs discovered on-the-fly as files are opened.

### Hook System (`src/schemas/hooks.ts`)

Hook types:
- `BashCommandHook` — `{type: 'command', command, if?, shell?, timeout?, once?, async?}`
- `PromptHook` — `{type: 'prompt', prompt, if?, timeout?}`
- `AgentHook` — agent-based hook

Events: `PreToolUse`, `PostToolUse`, `Notification`, `Stop`, `SubagentStop`.
Hooks support `if:` conditions using permission-rule syntax.

---

## 5. Config Hierarchy

### File Paths

| Source | Path | Priority |
|--------|------|----------|
| policySettings | `/etc/claude-code/managed-settings.json` | Highest |
| userSettings | `~/.claude/settings.json` | |
| projectSettings | `.claude/settings.json` | |
| localSettings | `.claude/settings.local.json` | |
| flagSettings | `--flag-settings-path` CLI arg | Lowest |

Merge: `userSettings → projectSettings → localSettings → policySettings` (policy wins).

### Key Settings Fields

- `permissions.allow/deny/ask` — permission rules
- `permissions.defaultMode` — `default`, `acceptEdits`, `bypassPermissions`, `plan`
- `autoCompactEnabled`, `autoMemoryEnabled`, `autoMemoryDirectory`
- `hooks` — HooksSettings with events as keys
- `mcpServers` — Record of MCP server configs
- `claudeMdExcludes` — glob patterns for memory exclusion
- `env` — environment variables to inject
- `model`, `theme`, `language`, `preferredNotifChannel`

### Key Environment Variables

| Variable | Purpose |
|----------|---------|
| `ANTHROPIC_API_KEY` | API authentication |
| `CLAUDE_CONFIG_DIR` | Override `~/.claude` |
| `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` | Disable memory |
| `DISABLE_COMPACT=1` | Disable all compaction |
| `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` | Custom compact threshold % |
| `CLAUDE_CODE_MAX_CONTEXT_TOKENS` | Override context window |
| `CLAUDE_CODE_TMPDIR` | Override scratchpad temp dir |
