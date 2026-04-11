# Four-Way Comparison: Claude Code vs Codex vs IronClaw vs cisco-code

Comprehensive subsystem-by-subsystem comparison of the four leading AI coding agent implementations. Goal: ensure cisco-code's architecture takes the best from each.

> **Date**: 2026-04-03
>
> **Sources**:
> - **Claude Code v2.1.88** (Anthropic) — TypeScript/Bun, 1,884 files, React/Ink TUI
> - **Codex** (OpenAI) — Rust 2024 + TS shim, 71 crates, Ratatui TUI
> - **IronClaw v0.17.0** (Near AI) — Rust 2024 + WASM tools/channels, PostgreSQL backend
> - **cisco-code** (Cisco) — Rust 2024, 12 crates, 97 .rs files, Ratatui TUI
>
> Also references **Claw-Code-Parity** (community Rust reimplementation of Claude Code, 8 crates, 46 .rs files) where relevant.

---

## 0. Executive Summary

| Metric | Claude Code | Codex | IronClaw | cisco-code |
|--------|:----------:|:-----:|:--------:|:----------:|
| **Owner** | Anthropic | OpenAI | Near AI | Cisco |
| **Language** | TypeScript (Bun) | Rust 2024 + TS shim | Rust 2024 + WASM | Rust 2024 |
| **Total files** | 1,884 .ts/.tsx | 71 crates | ~30 .rs + WASM components | 97 .rs + 10 .md skills |
| **LOC (approx)** | ~200K+ | ~100K+ | ~30K+ | ~25K |
| **Rust edition** | N/A | 2024 | 2024 (root), 2021 (WASM) | 2024 |
| **Architecture** | Monolith bundle | Workspace (71 crates) | Workspace + WASM components | Workspace (12 crates) |
| **Process model** | Single Bun process | Single binary | Single binary + Docker | Single binary |
| **UI framework** | React/Ink (331 components) | **Ratatui + Crossterm** | Raw terminal | **Ratatui + Crossterm** |
| **Tools** | 43 | ~30+ | WASM-sandboxed | **32** |
| **Commands** | 87 | ~20+ | ~15 | **28+** |
| **Bundled skills** | 17 | Yes (skills crate) | 0 | **10** |
| **Providers** | 4 (Anthropic, Bedrock, Vertex, Foundry) | 4+ (ChatGPT, LMStudio, Ollama, connectors) | Via MCP | **5** (Anthropic, OpenAI, Bedrock, Azure, OpenAI-compat) |
| **MCP transports** | 3 (stdio, SSE, HTTP) | MCP server + RMCP | MCP client | 2 (stdio, HTTP+SSE) |
| **Sandbox** | Library (`@anthropic-ai/sandbox-runtime`) | **OS-native** (bwrap, Seatbelt, Windows tokens) | **WASM + Docker** | OS-native (Seatbelt, Bubblewrap, Docker) |
| **Permission model** | 7 modes + ML classifier | Execpolicy rules + approval cache | **Capability-based** (WASM grants) | 4 modes + path rules + patterns |
| **Channels** | CLI only | CLI only | **7** (REPL, HTTP, Slack, Discord, Telegram, WhatsApp, web) | **3** (REPL, Slack, Webex) |
| **Storage** | JSONL files | JSONL files | **PostgreSQL** (pgvector) | JSONL + .meta.json |
| **Leak detection** | No | No | **Yes** (Aho-Corasick) | No |
| **Credential protection** | Settings denyWrite | Protected subpaths | **WASM-opaque injection** | N/A |
| **Build system** | npm/Bun | Bazel + Cargo | Cargo | Cargo |

### Strengths At a Glance

| Project | Best At |
|---------|---------|
| **Claude Code** | Prompt engineering depth (250+ fragments), ML permission classifier, services layer (22 categories), UI richness (331 components), team/teammate protocol |
| **Codex** | OS-native sandboxing (3 platforms), execpolicy rule language, network proxy isolation, in-process typed channels (zero serialization), scale (71 crates) |
| **IronClaw** | Zero-trust security (WASM isolation, capability grants, leak detection, credential injection), multi-channel (7 channels), prompt injection defense |
| **cisco-code** | Provider breadth (5 types), enterprise features (Webex, SQLite audit, server mode), compile-time skills (`include_str!()`), session metadata richness, plugin system |

---

## 1. Architecture

### 1.1 Crate/Module Structure

**Claude Code (TypeScript)** — Monolith:
```
src/
├── tools/          (43 dirs, ~178 files)
├── commands/       (87 dirs, ~189 files)
├── components/     (30 dirs, ~331 files)
├── services/       (22 dirs, ~130 files)
├── hooks/          (101 files)
├── utils/          (31 dirs, ~542 files)
├── skills/         (17 bundled)
├── ink/            (96 files — custom Ink terminal UI)
├── bridge/         (31 files — IDE connection)
└── ...34 more top-level dirs
```

**Codex (Rust + TS shim)** — 71-crate workspace:
```
codex-rs/
├── cli/            (entry point, subcommands)
├── tui/            (Ratatui interactive UI, ~311KB app.rs)
├── core/           (agent loop, seatbelt sandbox)
├── app-server/     (in-process runtime, message processor)
├── app-server-protocol/ (JSON-RPC 2.0, typed enums)
├── protocol/       (permissions, sandbox policy types)
├── exec/           (command execution + sandbox wrapping)
├── execpolicy/     (Starlark-like rule engine)
├── linux-sandbox/  (bwrap + seccomp)
├── windows-sandbox-rs/ (tokens + ACLs + desktop isolation)
├── mcp-server/     (MCP server implementation)
├── rmcp-client/    (RMCP client)
├── network-proxy/  (TCP→UDS→TCP bridge)
├── skills/         (skill system)
├── config/         (configuration hierarchy)
├── state/          (session state)
├── otel/           (OpenTelemetry integration)
├── chatgpt/, lmstudio/, ollama/ (provider connectors)
└── utils/ (15+ utility crates)
```

**IronClaw (Rust + WASM)** — Main binary + WASM components:
```
src/
├── tools/wasm/     (WASM host, capabilities, credential injection)
├── channels/wasm/  (WASM channel host)
├── sandbox/        (Docker container + HTTP proxy)
├── safety/         (leak detection, prompt injection defense)
├── ...
channels-src/       (compiled to .wasm, edition 2021)
├── slack/          discord/  telegram/  whatsapp/
tools-src/          (compiled to .wasm, edition 2021)
├── github/  gmail/  google-*  slack/  telegram/  web-search/
wit/                (WebAssembly Interface Types)
├── tool.wit        channel.wit
```

**cisco-code (Rust)** — 12-crate workspace:
```
crates/
├── runtime/        (22 files — agent loop, session, hooks, permissions, channels)
├── tools/          (22 files — 32 tools, registry pattern)
├── api/            (8 files — HTTP, OAuth, SSE, Bedrock SigV4)
├── cli/            (6 files — Ratatui TUI)
├── server/         (6 files — Axum REST + WebSocket)
├── mcp/            (5 files — MCP client)
├── sandbox/        (6 files — Seatbelt, Bubblewrap, Docker)
├── telemetry/      (5 files — SQLite audit, metrics, tracing)
├── plugin/         (5 files — plugin manifest, discovery)
├── providers/      (6 files — model routing, multi-provider)
├── protocol/       (5 files — shared types)
└── planning/       (1 file — placeholder)
```

### 1.2 Design Patterns

| Pattern | Claude Code | Codex | IronClaw | cisco-code |
|---------|:---------:|:-----:|:--------:|:----------:|
| Provider abstraction | Hard-coded Anthropic | Provider connectors (ChatGPT, Ollama, etc.) | Via MCP | Generic `P: Provider` |
| Tool execution | Sequential loop | `CodexMessageProcessor` | WASM component call | `Tool` trait + `ToolRegistry` |
| Tool isolation | Sandbox library | OS-native sandbox per command | **WASM Component Model** | OS-native sandbox |
| Config hierarchy | `.claude/` single scope | Multi-level merge | Config file | User/Project merge |
| Internal protocol | Direct function calls | **JSON-RPC 2.0 typed enums** (zero-copy in-process) | Direct + WASM boundary | Direct method calls |
| Skill embedding | Runtime TS functions | Skills crate | N/A | **`include_str!()` compile-time** |
| Frontend–backend | Same process (React/Ink) | **In-process typed channels** (no serialization) | Same process | Same process |
| External API | Bridge protocol | **app-server (stdio:// or ws://)** | HTTP gateway | **Axum REST + WebSocket** |

### 1.3 Rust Edition Comparison

| | Codex | IronClaw | cisco-code |
|--|:-----:|:--------:|:----------:|
| Root edition | 2024 | 2024 | 2024 |
| All crates | 2024 | Mixed (WASM = 2021) | 2024 |
| Why mixed? | N/A | wasm32 target compat | N/A |
| Generator-ready | Yes | Partial | Yes |
| Resolver | v3 | v3 (root) | v3 |

---

## 2. Agent Loop & Core Runtime

| Aspect | Claude Code | Codex | IronClaw | cisco-code |
|--------|------------|-------|----------|-----------|
| **Entry point** | `QueryEngine.ts` (68KB) | `CodexMessageProcessor` | Custom loop | `ConversationRuntime<P>` |
| **Generic over provider** | No | Connectors (ChatGPT, Ollama) | MCP-based | **Yes (`P: Provider`)** |
| **Streaming** | SSE → React state | SSE → typed channel → TUI | SSE → terminal | SSE → event emission |
| **Tool execution** | Sequential for loop | Sequential + sandbox wrap | WASM call | Sequential + hook pipeline |
| **Stop conditions** | end_turn, tool_use, max_tokens | end_turn, tool_use | end_turn | end_turn, tool_use, max_tokens |
| **Budget tracking** | Per-model rates, cumulative | Token counting | N/A | **Per-model cost + metadata** |
| **Doom loop detection** | Counter + injection | Not found | Not found | Not yet |
| **Interrupt handling** | Esc → cancel | TUI keybinding | N/A | Not yet |
| **Context injection** | 37 system reminder types | Limited | XML boundary wrapping | Partial (skill expansion, memory) |
| **Compaction** | LLM summary (~75% window) | Auto at 100K tokens | N/A (PostgreSQL) | **LLM summary + CompactBoundary marker** |
| **Scratchpad** | Per-session auto | Not found | N/A | **Per-session directory** |
| **Session fork** | UUID-based | Fork subcommand | N/A | **`Session::fork()` + metadata** |

### State Management Depth

| State Surface | Claude Code | Codex | IronClaw | cisco-code |
|--------------|:-----------:|:-----:|:--------:|:----------:|
| Cost tracking | Full (per-model, per-turn) | Token counting | N/A | **Per-model in metadata** |
| Turn metrics | Tool/hook/classifier duration | Iteration count | N/A | Turn count, compaction count |
| Skill/plugin state | Full lifecycle | Skills crate | N/A | **Bundled + discovered** |
| Plan mode | Full (3 recovery paths) | Not found | N/A | **Tool-based + skill** |
| Telemetry | OpenTelemetry counters | **OpenTelemetry (otel crate)** | Structured logging | **SQLite audit + spans** |

---

## 3. Provider / API Layer

| Provider | Claude Code | Codex | IronClaw | cisco-code |
|----------|:---------:|:-----:|:--------:|:----------:|
| Anthropic (direct) | Yes | Via connectors | Via MCP | **Yes** |
| AWS Bedrock | Yes | No | No | **Yes (SigV4)** |
| Google Vertex AI | Yes | No | No | No |
| Azure OpenAI | Yes (proxy) | No | No | **Yes** |
| OpenAI (direct) | No | **Yes (ChatGPT)** | No | **Yes** |
| OpenAI-compatible | No | **Yes (Ollama, LMStudio)** | No | **Yes (Groq, Together, Ollama)** |
| Foundry | Yes | No | No | No |
| XAI / Grok | No | No | No | No |
| **Total** | **4** | **4+** | Via MCP | **5** |

| Feature | Claude Code | Codex | IronClaw | cisco-code |
|---------|:---------:|:-----:|:--------:|:----------:|
| SSE streaming | Yes | Yes | N/A | Yes |
| OAuth flow | Full (PKCE, refresh) | Yes | N/A | **Full (`CodexAuth`)** |
| Prompt caching | Yes (cache control) | Yes | N/A | Not yet |
| Cost calculation | Per-model rates | Basic | N/A | **Per-model (`CostTracker`)** |
| Model class routing | No | No | N/A | **Yes (Small/Medium/Large)** |
| Model aliasing | 16 files | Basic | N/A | Basic |
| Retry logic | Yes | Yes | N/A | Not yet |

**cisco-code's Model Class Routing** is unique — auto-routes sub-tasks (compaction, titles) to cheaper models. Neither Claude Code nor Codex has this.

---

## 4. Tools

### 4.1 Tool Inventory

| Tool | CC | Codex | IronClaw | cisco |
|------|:--:|:-----:|:--------:|:-----:|
| Bash/shell | Yes | Yes | Docker sandbox | Yes |
| Read/read_file | Yes | Yes | workspace-read (WASM) | Yes |
| Write/write_file | Yes | Yes | workspace-write (WASM) | Yes |
| Edit/edit_file | Yes | Yes | N/A | Yes |
| Glob/glob_search | Yes | Yes | N/A | Yes |
| Grep/grep_search | Yes | Yes | N/A | Yes |
| WebFetch | Yes | Yes | http-request (WASM) | Yes |
| WebSearch | Yes | Yes | WASM tool | Yes |
| Agent/sub-agent | Yes | Yes | N/A | Yes |
| Skill | Yes | Yes | N/A | Yes |
| EnterPlanMode | Yes | Yes | N/A | Yes |
| ExitPlanMode | Yes | Yes | N/A | Yes |
| SendMessage | Yes | Yes | emit-message (WASM) | Yes |
| AskUserQuestion | Yes | Yes | N/A | Yes |
| Sleep | Yes | Yes | N/A | Yes |
| LSP | Yes | No | N/A | Yes |
| NotebookEdit | Yes | Yes | N/A | Yes |
| ToolSearch | Yes | Yes | N/A | Yes |
| Cron/Schedule | Yes | Yes | N/A | Yes (3 tools) |
| Tasks (6 tools) | Yes | Yes | N/A | Yes (6 tools) |
| Worktree | Yes | No | N/A | Yes |
| Config | Yes | Yes | N/A | Yes |
| MCP resources | Yes | Yes (MCP server) | N/A | Yes |
| REPL | Yes | Yes | WASM-sandboxed | No |
| PowerShell | Yes | Yes | N/A | No |
| Team tools | Yes | No | N/A | No |
| **Approx total** | **43** | **30+** | **WASM-based (10+)** | **32** |

### 4.2 Tool Architecture

| Pattern | Claude Code | Codex | IronClaw | cisco-code |
|---------|:---------:|:-----:|:--------:|:----------:|
| Registry | Global `tools.ts` | Built-in + plugin | WASM component registry | **`ToolRegistry::with_builtins()`** |
| Per-tool prompts | **Yes (40+ prompt.ts)** | Yes | N/A (WASM schema) | Inline descriptions |
| Permission per tool | Inline | `PermissionLevel` | **Capability grants** | `PermissionLevel` enum |
| Deferred loading | **Yes (~15 behind BM25)** | No | N/A | No |
| Dynamic MCP tools | Yes | Yes (MCP server) | Yes | Yes |
| Plugin-provided | Yes | Yes | **WASM components** | Yes (manifest) |
| Sandbox per tool | Library-based | **OS-native per command** | **WASM isolation per call** | OS-native |

### 4.3 Tool Isolation Models

This is a fundamental architectural difference:

| Model | Used By | How It Works | Overhead | Security |
|-------|---------|-------------|:--------:|:--------:|
| **No isolation** | Claude Code (default) | Trust tool code, sandbox bash only | None | Low |
| **OS sandbox per command** | Codex, cisco-code | bwrap/Seatbelt wraps each shell command | Low (~5ms) | High |
| **WASM per call** | IronClaw | Each tool runs in WASM Component Model sandbox | Medium (~20ms cold) | **Highest** |
| **Docker container** | IronClaw (fallback), cisco-code | Full container per execution | High (~500ms) | High |

---

## 5. Sandboxing — Deep Comparison

### 5.1 Platform Coverage

| Platform | Claude Code | Codex | IronClaw | cisco-code |
|----------|:---------:|:-----:|:--------:|:----------:|
| **Linux** | sandbox-runtime lib | **Bubblewrap + Seccomp + namespaces** | Docker | Bubblewrap + Landlock |
| **macOS** | sandbox-runtime lib | **Seatbelt** (inline .sbpl profiles) | Docker | Seatbelt |
| **Windows** | sandbox-runtime lib | **Restricted tokens + ACLs + desktop isolation** | Docker | N/A |
| **Container** | No | No | **Docker (non-root, dropped caps, RO root)** | Docker |
| **WASM** | No | No | **Component Model (strongest isolation)** | No |

### 5.2 Filesystem Isolation

| Feature | Claude Code | Codex | IronClaw | cisco-code |
|---------|:---------:|:-----:|:--------:|:----------:|
| Default filesystem | Library-controlled | **RO root, explicit writable roots** | WASM: no FS; Docker: RO root | Configurable |
| Protected subpaths | `.claude/`, `.git/` (denyWrite) | **`.git/hooks`, `.codex/` RO within writable** | WASM: prefix validation | Not yet |
| Path traversal defense | Case-insensitive normalize | bwrap mount ordering | **Multi-layer validation (WASM + Docker)** | Regex rules |
| Write scoping | allowWrite paths | Writable roots + deny subpaths | `workspace-write` (channel-namespaced prefix) | Path rules |

**Codex is best here** — its mount ordering (RO root → writable roots → protected subpaths → nested unreadables) is the most thorough. cisco-code should adopt protected subpaths within writable roots.

### 5.3 Network Isolation

| Feature | Claude Code | Codex | IronClaw | cisco-code |
|---------|:---------:|:-----:|:--------:|:----------:|
| Isolation method | Domain allowlist | **Network namespace + managed proxy** | **HTTP proxy with domain allowlist** | Policy enum (None/WorkspaceOnly/Allowlist/Full) |
| Linux network NS | No | **Yes (`--unshare-net`)** | Docker network | No |
| Proxy architecture | N/A | **TCP→UDS→TCP bridge** | **All container traffic through HTTP proxy** | N/A |
| Seccomp socket block | No | **Yes (blocks new AF_UNIX in proxy mode)** | N/A | No |
| Domain validation | allowedDomains list | **Network rules (host + protocol + method)** | **Allowlist + credential auto-inject** | Allowlist config |

**Codex is best for Linux** (namespace + seccomp). **IronClaw is best for containers** (proxy validates every request).

### 5.4 Credential Protection

| Feature | Claude Code | Codex | IronClaw | cisco-code |
|---------|:---------:|:-----:|:--------:|:----------:|
| Secret exposure | Settings file denyWrite | Protected paths | **WASM-opaque (tool never sees values)** | N/A |
| Injection method | N/A | N/A | **Host injects into HTTP headers at boundary** | N/A |
| Locations | N/A | N/A | Bearer, Basic, Header, QueryParam, UrlPath | N/A |
| Existence check | N/A | N/A | **`secret-exists()` only (never read)** | N/A |

**IronClaw is best** — tools literally cannot access secret values, only check existence. Secrets are injected by the host at the HTTP boundary.

### 5.5 Leak Detection

| Feature | Claude Code | Codex | IronClaw | cisco-code |
|---------|:---------:|:-----:|:--------:|:----------:|
| Output scanning | No | No | **Yes (Aho-Corasick + regex)** | No |
| Patterns detected | N/A | N/A | OpenAI keys, Anthropic keys, AWS keys, GitHub tokens, PEM keys, DB URIs | N/A |
| Scan points | N/A | N/A | **4: inbound, outbound WASM, outbound container, tool outputs** | N/A |
| Actions | N/A | N/A | Block (critical), Redact (high), Warn (medium) | N/A |

**IronClaw is uniquely strong** here. All other implementations lack leak detection entirely.

---

## 6. Permission System — Deep Comparison

### 6.1 Permission Modes

| Mode | Claude Code | Codex | IronClaw | cisco-code |
|------|:---------:|:-----:|:--------:|:----------:|
| Read-only | Yes | Yes (`ReadOnly` sandbox) | Per-capability | Yes (`accept-reads`) |
| Workspace write | Yes | Yes (`WorkspaceWrite` sandbox) | Per-capability | Yes |
| Full/bypass | Yes | Yes (`DangerFullAccess`) | `FullAccess` (no sandbox) | Yes (`bypass`) |
| Default/prompt | Yes | Yes (`UnlessTrusted`) | N/A (capability deny-default) | Yes (`default`) |
| Deny-all | Yes (`dontAsk`) | Yes (`Never`) | Default (no caps) | Yes (`deny-all`) |
| Accept-edits | Yes | Yes (`OnRequest`) | N/A | No |
| Plan mode | Yes | No | N/A | Yes |
| **Auto (ML classifier)** | **Yes (Haiku)** | No | No | No |
| **Total** | **7** | **5** | **Capability-based** | **4** |

### 6.2 Permission Decision Flow

**Claude Code** (most sophisticated decision logic):
```
1. Deny rules (absolute, bypass-immune) → DENY
2. Ask rules (tool-level) → ASK
3. Tool.checkPermissions() → ALLOW/ASK/DENY
4. Content-specific ask rules (Bash(npm publish:*)) → ASK
5. Safety checks (.git/, .claude/, shell configs) → ASK (bypass-immune)
6. Mode check:
   - bypassPermissions → ALLOW
   - dontAsk → DENY
   - auto → ML classifier
   - default → prompt user
```

**Codex** (execpolicy + approval cache):
```
1. Execpolicy prefix rules → allow/prompt/forbidden
2. Approval check:
   - Never → auto-reject
   - OnFailure → auto-approve
   - UnlessTrusted → prompt unless safe
   - Granular → per-category check
3. Session approval cache → skip if previously approved
4. Sandbox override for first attempt
```

**IronClaw** (capability-based, deny-default):
```
1. Check capability grants:
   - workspace_read: allowed prefixes?
   - http: endpoint allowlist? rate limit?
   - tool_invoke: aliased? rate limit?
   - secrets: glob pattern match?
2. No capability → denied (zero-trust)
3. Per-execution rate limits enforced
```

**cisco-code** (mode + rules + patterns):
```
1. Mode check (default/accept-reads/bypass/deny-all)
2. Tool-level overrides (AlwaysAllow/AlwaysDeny/Default)
3. Path rules (regex allow/deny)
4. Dangerous pattern detection (rm -rf, force-push, DROP, chmod)
5. Prompt user or auto-decide
```

### 6.3 Permission Infrastructure

| Component | Claude Code | Codex | IronClaw | cisco-code |
|-----------|:---------:|:-----:|:--------:|:----------:|
| Rule engine | 7-source hierarchy | **Starlark-like prefix rules** | Capability struct | Mode + overrides |
| ML classifier | **Yes (Haiku auto-mode)** | No | No | No |
| Dangerous patterns | `dangerousPatterns.ts` | N/A (execpolicy rules) | N/A (WASM blocks all) | **Yes (regex)** |
| Bash classifier | `bashClassifier.ts` (ML) | N/A | N/A | No |
| Denial tracking | 3x auto-deny | Approval cache | N/A | No |
| Path validation | `pathValidation.ts` + case-insensitive | Protected subpaths | **Multi-layer (WASM + Docker)** | Regex allow/deny |
| Rule sources | policy→flags→user→project→local→CLI→session | Rules files + config | capabilities.json per tool | Config + CLI |

### 6.4 What cisco-code Should Adopt

| From | Feature | Why | Priority |
|------|---------|-----|:--------:|
| **Codex** | Protected subpaths in writable roots | `.git/hooks` and `.cisco-code/` must stay RO even when CWD is writable | **P0** |
| **Codex** | Execpolicy prefix rules | Starlark-like `prefix_rule(["git","push","--force"], decision="forbidden")` is more expressive than regex | P1 |
| **Codex** | Approval cache | Skip re-prompting for previously approved actions | P1 |
| **IronClaw** | Leak detection (Aho-Corasick) | Scan tool outputs for API keys/secrets before returning to LLM | **P1** |
| **IronClaw** | Credential injection at boundary | For server mode: inject secrets into HTTP requests without exposing to agent | P2 |
| **Claude Code** | 7-source rule hierarchy | policy > flags > user > project > local > CLI > session | P2 |
| **Claude Code** | Bypass-immune deny/ask rules | Deny rules must work even in bypass mode | P1 |
| **Claude Code** | Denial tracking (3x auto-deny) | Prevent infinite re-prompting loops | P2 |

---

## 7. Commands / Slash Commands

| Category | Claude Code | Codex | IronClaw | cisco-code |
|----------|:---------:|:-----:|:--------:|:----------:|
| Core (help, status, version, clear, cost) | All | Most | Basic | All |
| Session (resume, export, config, session) | 5 | 4 (resume, fork, apply) | N/A | 4 |
| Model/Permissions | 4 | 3 | N/A | 3 |
| MCP | Full CRUD | MCP server management | N/A | Full |
| Git (commit, pr, issue, diff) | 5 | Not found | N/A | 4 |
| Planning (plan, tasks, ultraplan) | 3 | Not found | N/A | 2 |
| Skills/Plugins | 3 | Skills support | N/A | 2 |
| Debug (debug-tool-call, bughunter, doctor) | 3 | Not found | N/A | 2 |
| IDE (ide, desktop, mobile) | 3 | No | No | No |
| Voice | 1 | No | No | No |
| Vim/keybindings | 2 | No | No | No |
| Review (review, autofix-pr) | 3 | No | No | No |
| Remote (bridge, remote-setup) | 3 | Not found | No | No |
| **Approx total** | **87** | **~20+** | **~15** | **28+** |

---

## 8. Prompt Assembly

| Aspect | Claude Code | Codex | IronClaw | cisco-code |
|--------|------------|-------|----------|-----------|
| **Assembly** | `DX()` monolith, 17 sections, ~25K tokens | SystemPrompt in core | Basic system prompt | `build_system_prompt()` + skills + memory |
| **Fragment count** | **~250 across tools/skills/modes** | Per-tool prompt fragments | Minimal | Inline + bundled skill content |
| **Per-tool prompts** | **40+ prompt.ts files** | Yes (per-tool) | N/A | Not yet |
| **Instruction files** | CLAUDE.md, rules/*.md (8K/file, 24K total) | config-based | N/A | CLAUDE.md, .assistant.md |
| **Cache boundary** | `SYSTEM_PROMPT_DYNAMIC_BOUNDARY` | Not found | N/A | Not yet |
| **Memory injection** | Yes (session + persistent) | Not found | N/A | **Yes** |
| **Skill listing** | Yes | Yes | N/A | **Yes** |
| **Git context** | Yes (status, diff) | Not found | N/A | **Yes** |
| **Anti-patterns** | Yes (6 rules) | Not found | N/A | Not yet |
| **Safety section** | Yes ("executing with care") | Not found | N/A | Not yet |
| **Mid-conversation reminders** | **37 types** | Not found | N/A | Partial |

**Claude Code dominates** in prompt depth. The 250+ fragments and 40+ per-tool prompt files are the single highest-impact factor for agent quality.

---

## 9. Session & Persistence

| Aspect | Claude Code | Codex | IronClaw | cisco-code |
|--------|:---------:|:-----:|:--------:|:----------:|
| **Format** | JSONL | JSONL | **PostgreSQL** (pgvector) | JSONL + .meta.json |
| **Message types** | 8+ | Typed protocol enums | DB records | **6 (incl. CompactBoundary)** |
| **Metadata sidecar** | `.meta.json` (some versions) | Not found | DB columns | **`.meta.json` (always)** |
| **Fork support** | UUID copy | `fork` subcommand | N/A | **`Session::fork()` + forked_from** |
| **Compaction** | LLM summary at ~75% | LLM summary at 100K | N/A (DB) | **LLM summary + boundary markers** |
| **Session listing** | Rich | Basic | DB query | **Rich (name, cost, turns, first prompt)** |
| **Session naming** | Auto + manual | Not found | N/A | **Auto + `set_name()`** |
| **Vector search** | No | No | **Yes (pgvector)** | No |

cisco-code has the richest session metadata of any file-based implementation. IronClaw's PostgreSQL approach enables vector search but requires a database dependency.

---

## 10. Skills System

| Aspect | Claude Code | Codex | IronClaw | cisco-code |
|--------|:---------:|:-----:|:--------:|:----------:|
| Bundled skills | 17 (TS, feature-gated) | Yes (skills crate) | 0 | **10 (`include_str!()`)** |
| Discovery | User > project > community > built-in > MCP | Config-based | N/A | **User > project > bundled** |
| Feature gating | Yes (KAIROS, AGENT_TRIGGERS) | Not found | N/A | No |
| Skill format | TS functions | Rust modules | N/A | **Markdown + YAML frontmatter** |
| Expansion mechanism | SkillTool → prompt injection | Built-in | N/A | **`expand_skill_result()` with `<command-name>` tags** |
| User-invocable flag | Yes | Not found | N/A | **Yes (frontmatter)** |

---

## 11. MCP (Model Context Protocol)

| Aspect | Claude Code | Codex | IronClaw | cisco-code |
|--------|:---------:|:-----:|:--------:|:----------:|
| **Role** | MCP client | **MCP server** + RMCP client | MCP client | MCP client |
| **Transports** | 3 (stdio, SSE, HTTP) | stdio + more | Stdio | 2 (stdio, HTTP+SSE) |
| **Tool wrapping** | Yes (MCPTool, ListMcp, ReadMcp) | MCP server exposes tools | N/A | **Yes (ListMcp, ReadMcp)** |
| **Tool prefixing** | `mcp__[server]__[tool]` | N/A (is the server) | N/A | `mcp__[server]__[tool]` |
| **Hot reload** | Yes | Not found | N/A | No |
| **OAuth** | Yes | Not found | N/A | No |

**Codex is unique** in being an MCP **server** (exposing its tools to other agents) rather than just a client. This is worth considering for cisco-code's server mode.

---

## 12. Hooks System

| Event | Claude Code | Codex | IronClaw | cisco-code |
|-------|:---------:|:-----:|:--------:|:----------:|
| PreToolUse | Yes | Not found (execpolicy instead) | PreToolUse (WASM capability check) | **Yes** |
| PostToolUse | Yes | Not found | PostToolUse (WASM) | **Yes** |
| PostToolUseFailure | Yes | Not found | PostToolUseFailure (WASM) | No |
| SessionStart | Yes | Not found | on-start (WASM) | **Yes** |
| SessionEnd | Yes | Not found | on-shutdown (WASM) | **Yes** |
| PreMessage | Yes | Not found | N/A | **Yes** |
| PostMessage | Yes | Not found | N/A | **Yes** |
| PreCompact | Yes | Not found | N/A | No |
| ConfigChange | Yes | Not found | N/A | No |
| **Total** | **~10** | **0 (rules-based)** | **3 WASM events** | **6** |

Codex replaces hooks with execpolicy rules — a different philosophy. IronClaw uses WASM lifecycle events. cisco-code is closest to Claude Code's hook model.

---

## 13. Channels / Multi-Interface

| Channel | Claude Code | Codex | IronClaw | cisco-code |
|---------|:---------:|:-----:|:--------:|:----------:|
| CLI/REPL | Yes | Yes | Yes | **Yes** |
| HTTP/Web gateway | No | No | **Yes** | No |
| Slack | No | No | **Yes (WASM)** | **Yes (native)** |
| Webex | No | No | No | **Yes (native)** |
| Discord | No | No | **Yes (WASM)** | No |
| Telegram | No | No | **Yes (WASM)** | No |
| WhatsApp | No | No | **Yes (WASM)** | No |
| IDE/WebSocket | Yes (bridge) | Yes (ws://) | No | **Yes (Axum WS)** |
| **Total** | **2** | **2** | **7** | **4** |

IronClaw leads in channel count. cisco-code is second with Slack + Webex + REPL + server mode.

**Planned additions for cisco-code**: Notion (polling + REST), Obsidian (file watcher).

---

## 14. UI / Terminal

| Aspect | Claude Code | Codex | IronClaw | cisco-code |
|--------|:---------:|:-----:|:--------:|:----------:|
| **Framework** | React/Ink (331 components) | **Ratatui 0.29 + Crossterm** | Raw terminal | **Ratatui + Crossterm** |
| **Component count** | 331+ | TUI with chatwidget, approval pane | Minimal | 4 TUI modules |
| **Themes** | Multiple, configurable | Not found | No | `/theme` command |
| **Vim mode** | Yes (7 files) | Not found | No | No |
| **Voice** | Yes (3 files) | No | No | No |
| **IDE integration** | Bridge protocol (31 files) | **app-server (stdio:// or ws://)** | HTTP gateway | **Axum REST + WS** |
| **Streaming display** | React state → virtual DOM | **Typed channel → ratatui** | Terminal print | Event → ratatui |

**Codex and cisco-code share the same TUI approach** (Ratatui + Crossterm). Codex's `app.rs` is ~311KB — much more mature. cisco-code should study its chatwidget and approval pane implementations.

---

## 15. Telemetry & Observability

| Aspect | Claude Code | Codex | IronClaw | cisco-code |
|--------|:---------:|:-----:|:--------:|:----------:|
| Framework | Custom + analytics | **OpenTelemetry (dedicated otel crate)** | Structured logging | **SQLite audit + custom spans** |
| Metrics | Per-model counters | OTel meters/counters | N/A | Token/latency metrics |
| Tracing | OpenTelemetry loggers | **OTel spans with W3C trace context** | N/A | Span hierarchy (Interaction→LLM→Tool) |
| Audit | Analytics sink | Not found | N/A | **SQLite (enterprise compliance)** |
| Export | N/A | Not found | N/A | **JSON/CSV export** |

Codex has the best standards-compliant telemetry (OpenTelemetry). cisco-code has the best enterprise audit (SQLite). Consider adding OTel export alongside SQLite.

---

## 16. Unique Features by Project

### Claude Code Only
| Feature | Description | Adopt? |
|---------|-------------|:------:|
| ML permission classifier | Haiku evaluates risk per-call | P3 (complex) |
| 250+ prompt fragments | Per-tool prompt engineering | **P0** |
| Teammate protocol | Persistent multi-agent teams | P3 |
| Voice input | Speech-to-text | P3 |
| Vim mode | TUI vim keybindings | P3 |
| Services layer (22 categories) | Analytics, memory sync, tips, etc. | P2 |
| Deferred tool loading (BM25) | Save ~2-3K tokens | P2 |
| 37 system reminder types | Mid-conversation context injection | P2 |

### Codex Only
| Feature | Description | Adopt? |
|---------|-------------|:------:|
| Execpolicy rule language | Starlark-like prefix matching | **P1** |
| 71-crate workspace | Extreme modularity | No (over-engineering for our scale) |
| Windows sandbox (tokens + ACLs) | Full Windows support | P3 |
| Network namespace + seccomp proxy | Strongest Linux network isolation | **P1** |
| Protected subpaths in writable roots | `.git/hooks` stays RO | **P0** |
| In-process typed channels | Zero-serialization frontend-backend | Consider |
| Approval cache per session | Skip re-prompting | **P1** |
| MCP server mode | Expose tools to other agents | P2 |
| OpenTelemetry with W3C trace context | Standards-compliant tracing | P2 |
| macOS Seatbelt extensions | Calendar, contacts, automation | P3 |

### IronClaw Only
| Feature | Description | Adopt? |
|---------|-------------|:------:|
| WASM Component Model sandbox | Strongest tool isolation | P3 (for untrusted plugins only) |
| Credential injection at boundary | WASM-opaque secrets | **P2** |
| Leak detection (Aho-Corasick) | Scan all I/O for API keys | **P1** |
| 7 channels (Slack, Discord, Telegram, WhatsApp, web) | Broadest reach | Study patterns |
| PostgreSQL + pgvector | Vector search over sessions | P3 |
| Prompt injection defense (XML boundary wrapping) | Mark trusted/untrusted boundaries | **P1** |
| Per-execution fresh state | No carryover between tool calls | P2 |
| Per-execution rate limits | HTTP requests (50), tool invocations (20) | P2 |
| Webhook signature verification | Ed25519, HMAC-SHA256 | P2 (for channels) |

### cisco-code Only
| Feature | Description | Advantage |
|---------|-------------|-----------|
| Webex channel | Cisco enterprise integration | Enterprise deployment |
| Plugin system (crate) | TOML/JSON manifest, discovery | Extensibility |
| Sandbox crate (3 strategies) | Seatbelt + Bubblewrap + Docker | Cross-platform |
| Server crate (Axum) | REST + WebSocket + job queue | IDE/remote integration |
| SQLite audit logging | Enterprise compliance | Observability |
| Model class routing (S/M/L) | Auto-route sub-tasks to cheaper models | Cost optimization |
| `include_str!()` skills | Zero-cost compile-time embedding | Performance |
| Session metadata sidecar | Rich `.meta.json` | Session management |
| Cron system + task system | Recurring jobs + dependencies | Automation |
| Channel manager | Multi-channel routing (REPL, Slack, Webex) | Unified agent |

---

## 17. Recommended Hybrid Architecture for cisco-code

Based on all four projects, the optimal architecture picks the best approach per layer:

```
┌─────────────────────────────────────────────────────────────────┐
│                         CLI (Ratatui)                           │
│          Same approach as Codex — study their chatwidget        │
├─────────────────────────────────────────────────────────────────┤
│                    PERMISSION ENGINE                             │
│  Claude Code: bypass-immune deny rules, 7-source hierarchy     │
│  + Codex: execpolicy prefix rules, approval cache              │
│  + IronClaw: leak detection on all tool outputs                 │
├─────────────────────────────────────────────────────────────────┤
│                    RUNTIME / AGENT LOOP                          │
│  cisco-code: ConversationRuntime<P>, hook pipeline              │
│  + Claude Code: per-tool prompt fragments, system reminders     │
│  + Claude Code: doom loop detection                             │
├─────────────────────────────────────────────────────────────────┤
│                       SANDBOX                                    │
│  Codex: OS-native (bwrap/Seatbelt) + protected subpaths        │
│  + Codex: network namespace + managed proxy (Linux)             │
│  + IronClaw: WASM isolation for untrusted plugins only          │
│  + IronClaw: credential injection at sandbox boundary           │
├─────────────────────────────────────────────────────────────────┤
│                      PROVIDERS                                   │
│  cisco-code: 5 providers + model class routing (keep)           │
│  + Codex: connector pattern for new providers                   │
├─────────────────────────────────────────────────────────────────┤
│                      CHANNELS                                    │
│  cisco-code: native Rust channels (keep, simpler than WASM)     │
│  + IronClaw: study Slack/Discord patterns                       │
│  + Add: Notion (poll), Obsidian (file watch)                    │
├─────────────────────────────────────────────────────────────────┤
│                     TELEMETRY                                    │
│  cisco-code: SQLite audit (keep for enterprise)                 │
│  + Codex: OpenTelemetry export (add alongside)                  │
├─────────────────────────────────────────────────────────────────┤
│                     MCP                                          │
│  cisco-code: client (keep) + add server mode (from Codex)       │
│  + Add WebSocket transport                                      │
└─────────────────────────────────────────────────────────────────┘
```

---

## 18. Prioritized Gap List

### P0 — Critical (do first)

| Gap | Source | What to Do | Effort |
|-----|--------|-----------|:------:|
| Per-tool prompt fragments | Claude Code | Create per-tool prompt files (Bash safety, git safety, Edit guidance) | Medium |
| Protected subpaths in writable sandbox | Codex | `.git/hooks`, `.cisco-code/` read-only within writable roots | Small |
| "Executing with care" prompt section | Claude Code | Port safety guidelines about reversibility, blast radius | Small |

### P1 — High Priority

| Gap | Source | What to Do | Effort |
|-----|--------|-----------|:------:|
| Leak detection | IronClaw | Aho-Corasick scanner for API keys in tool outputs | Medium |
| Prompt injection defense | IronClaw | XML boundary wrapping for untrusted content | Small |
| Execpolicy prefix rules | Codex | Command classification beyond regex patterns | Medium |
| Bypass-immune deny rules | Claude Code | Deny rules must work even in bypass mode | Small |
| Approval cache | Codex | Skip re-prompting for previously approved actions | Small |
| Network namespace isolation | Codex | `--unshare-net` + managed proxy for Linux sandbox | Medium |
| Prompt caching | Codex/Claude Code | Cache control headers, prompt cache config | Medium |

### P2 — Important

| Gap | Source | What to Do | Effort |
|-----|--------|-----------|:------:|
| Credential injection at boundary | IronClaw | For server mode: inject secrets into sandboxed requests | Medium |
| Doom loop detection | Claude Code | Counter + injection when agent repeats | Small |
| MCP server mode | Codex | Expose cisco-code tools via MCP to other agents | Medium |
| OpenTelemetry export | Codex | Add OTel alongside SQLite audit | Medium |
| 7-source rule hierarchy | Claude Code | policy > flags > user > project > local > CLI > session | Medium |
| Anti-pattern rules in prompt | Claude Code | Port 6 anti-pattern rules | Small |
| Deferred tool loading | Claude Code | ToolSearch-based BM25 to save tokens | Medium |
| Notion channel | New | Polling + REST for project management | Medium |
| Obsidian channel | New | File watcher for knowledge management | Small |

### P3 — Future

| Gap | Source | What to Do | Effort |
|-----|--------|-----------|:------:|
| WASM isolation for untrusted plugins | IronClaw | Component Model sandbox for community tools | Large |
| ML permission classifier | Claude Code | Auto-approve safe commands via smaller model | Large |
| Teammate protocol | Claude Code | Persistent multi-agent teams | Large |
| Windows sandbox | Codex | Restricted tokens + ACLs | Large |
| Voice input | Claude Code | Speech-to-text | Large |
| PostgreSQL backend | IronClaw | Optional DB storage + vector search | Large |
| Discord/Telegram/WhatsApp channels | IronClaw | Study WASM patterns, implement native | Medium each |
| Per-execution rate limits | IronClaw | Cap HTTP/tool calls per tool execution | Small |

---

## 19. Conclusion

**cisco-code occupies a strong middle ground** — more enterprise-ready than any competitor (Webex, SQLite audit, server mode, plugin system), with the broadest provider support (5 types) and model class routing that none of the others have.

**The path forward is clear**:
1. **Security**: Adopt Codex's protected subpaths and network isolation + IronClaw's leak detection and prompt injection defense
2. **Quality**: Port Claude Code's per-tool prompt fragments (highest-impact single improvement)
3. **Permissions**: Merge Codex's execpolicy rules with Claude Code's bypass-immune deny rules
4. **Channels**: Finish Slack/Webex polish, add Notion and Obsidian
5. **Interop**: Add MCP server mode (from Codex) so cisco-code can be consumed by other agents

| Metric | Claude Code | Codex | IronClaw | cisco-code |
|--------|:----------:|:-----:|:--------:|:----------:|
| **Best at** | Prompt depth, UX | Sandboxing, modularity | Zero-trust security | Enterprise, breadth |
| **Weakness** | No Rust, no sandbox depth | No channels, no skills | WASM overhead, small ecosystem | Prompt depth, leak detection |
| **Maturity** | Production (millions of users) | Production (OpenAI) | Early (v0.17) | Development |
| **Architecture fit for cisco-code** | Learn prompts & permissions | Learn sandbox & network | Learn security & channels | — |
