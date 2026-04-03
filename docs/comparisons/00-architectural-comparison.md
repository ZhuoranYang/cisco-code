# Architectural Comparison: Claude Code vs Codex vs Astro-Assistant vs Claw-Code-Parity

> This document provides a deep comparative analysis of four agentic coding harnesses,
> distilling the best architectural ideas that inform the design of **cisco-code**.

---

## 1. Overview

| Dimension | Claude Code v2.1.88 | Codex (OpenAI) | Astro-Assistant | Claw-Code-Parity |
|-----------|---------------------|-----------------|-----------------|-------------------|
| **Language** | TypeScript (Ink/React CLI) | Rust (70+ crates, Ratatui TUI) | Python 3.11+ (engine) + TS (Ink CLI) | Rust (37K LOC) + Python (reference) |
| **LOC** | ~800K+ (src/) | ~200K+ (codex-rs/) | ~52K Python + ~15K TS | ~37K Rust + ~2K Python |
| **Agent Loop** | Flat ReAct (QueryEngine) | Flat ReAct (state machine) | Two-level: DAG planner + ReAct inner | Flat ReAct (ConversationRuntime) |
| **LLM Providers** | Anthropic only | OpenAI + Ollama/LM Studio | 10 backends (Anthropic, OpenAI, Google, Bedrock, 14 OpenAI-compat) | Anthropic + OpenAI-compat |
| **Tools** | 45+ built-in + MCP | ~10 built-in + MCP | 35 built-in + MCP | 20+ built-in + MCP |
| **Sandboxing** | None (permission-based) | OS-native (Seatbelt, Bubblewrap, Windows tokens) | Designed (Docker stubs) | Container detection |
| **Model Routing** | Single model/session (fast toggle) | Single model/session | 5-tier routing (SMALL→FRONTIER) | Single model with aliases |
| **RL Traces** | No | No | Yes (state-action-observation-reward) | No |

---

## 2. Agent Loop Architecture

### 2.1 Claude Code: Streaming ReAct with Rich Tool Orchestration

```
User → QueryEngine.submitMessage()
  → Build system prompt + context
  → Call Anthropic API (streaming)
  → For each tool_use block:
      → canUseTool() permission check
      → Pre-hooks
      → tool.call()
      → Post-hooks
      → Yield progress events
  → Send tool results back to API
  → Loop until stop_reason = "end_turn"
```

**Key Insight:** Claude Code's strength is its **mature tool orchestration layer**. Every tool
gets a rich `ToolUseContext` with ~20+ fields (app state, file cache, abort controller, MCP
clients, agent definitions, notification system). This makes tools first-class citizens that
can influence the entire session.

**Design Pattern Worth Adopting:**
- Tool as the central abstraction (not the LLM call)
- Generator-based streaming (events yielded, not collected)
- `ToolResult` can include `newMessages` and `contextModifier` — tools reshape the conversation

### 2.2 Codex: State Machine with OS-Native Sandboxing

```
User → TUI/CLI
  → CodexThread (state machine)
      Initial → Processing → AwaitingApproval → Executing → Complete
  → ModelClient streams via SSE/WebSocket/REST (transport fallback)
  → ToolRegistry routes to handler
  → Sandbox transforms CommandSpec per platform
  → Execute in isolated subprocess
  → Return results → loop
```

**Key Insight:** Codex's defining feature is **security-first execution**. Every command runs
through a platform-specific sandbox transform:
- macOS: Seatbelt mandatory access control profiles
- Linux: Bubblewrap (user namespaces) + Landlock (LSM)
- Windows: Restricted tokens

The `SandboxTransformRequest` → `ExecRequest` pipeline ensures that **tool execution is
always mediated by the OS security model**, not just regex blocklists.

**Design Patterns Worth Adopting:**
- Transport fallback strategy (WebSocket → SSE → REST)
- Platform-specific sandbox profiles as first-class config
- Agent depth/count limits (max_depth=1, max_threads=6)
- Feature flag stages (Developmental → Experimental → Stable → Deprecated)

### 2.3 Astro-Assistant: Two-Level Hierarchical Planning

```
Task received
  → Complexity classifier (one SWIFT LLM call)
  ├─ Simple: direct ReAct (inner loop only)
  └─ Complex: plan mode
      → Explore + understand codebase
      → Build typed DAG with dependencies
      → Assign model tier per step
      → Execute parallel waves (independent tasks concurrent)
      → Each step = focused ReAct with scoped context
      → At milestones: reflect + assess
      → Replan if needed (retry, escalate tier, decompose further)
```

**Key Insight:** Astro's **two-level architecture beats flat ReAct for complex problems**:
1. Global context (all remaining tasks) enables better tool choices
2. Independent tasks run in parallel → faster execution
3. Model tier assignment → dramatic cost savings (20x difference possible)
4. Replanning on failure → adaptive behavior vs. retrying same approach

**Design Patterns Worth Adopting:**
- DAG-based task decomposition with topological execution
- 5-tier model routing (SMALL/MEDIUM/LARGE/TEAMMATE/FRONTIER)
- Composable policy engine for safety (tree of And/Or/Not policies)
- RL trajectory collection (state, action, observation, reward)
- Provider adapter ABC for clean multi-model support

### 2.4 Claw-Code-Parity: Clean-Room Rust Reimplementation

```
User → rusty-claude-cli (REPL)
  → ConversationRuntime<C: ApiClient, T: ToolExecutor>
      → Build system prompt + CLAUDE.md discovery
      → Stream via SSE
      → Parse tool_use → permission check → hook → execute → hook
      → Submit tool result → loop
```

**Key Insight:** Claw's generic `ConversationRuntime<C, T>` design is elegant — the runtime
is parameterized over the API client and tool executor traits. This enables:
- Swappable LLM backends (Anthropic, OpenAI-compat)
- Testable tool execution (mock executors)
- Clean separation of concerns

**Design Patterns Worth Adopting:**
- Trait-based generics for runtime (ApiClient + ToolExecutor)
- Modular Cargo workspace with clear crate boundaries
- JSONL session persistence (append-friendly)
- Config hierarchy (.claude.json at user/project/local scope)
- Clean-room approach to understanding proprietary architectures

---

## 3. Tool System Comparison

### 3.1 Tool Definition Models

**Claude Code (TypeScript — Zod schemas):**
```typescript
type Tool<Input, Output> = {
  inputSchema: ZodSchema<Input>    // Validation + JSON schema generation
  call(args, context): Promise<ToolResult<Output>>
  description(input): Promise<string>
  isReadOnly(input): boolean
  isDestructive?(input): boolean
  isConcurrencySafe(input): boolean
}
```

**Codex (Rust — trait-based):**
```rust
#[async_trait]
pub trait ToolHandler: Send + Sync {
    type Output: ToolOutput + 'static;
    fn kind(&self) -> ToolKind;
    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output>;
}
```

**Astro-Assistant (Python — registry-based):**
```python
# Tools registered via decorator with metadata
@tool(name="bash", permission=PermissionLevel.CONFIRM, timeout=300)
async def bash_tool(command: str, ctx: ToolContext) -> ToolResult:
    ...
```

**Claw-Code-Parity (Rust — spec + registry):**
```rust
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,    // JSON Schema
    pub required_permission: PermissionMode,
}
```

### 3.2 Tool Inventory Comparison

| Tool Category | Claude Code | Codex | Astro | Cisco-Code Target |
|--------------|-------------|-------|-------|-------------------|
| **Shell** | BashTool | shell | bash | bash, powershell |
| **File Read** | FileReadTool | (via shell) | read | read_file |
| **File Write** | FileWriteTool | (via shell) | write | write_file |
| **File Edit** | FileEditTool | apply_patch | edit | edit_file (str_replace) + apply_patch |
| **Search Files** | GlobTool | (via shell) | glob | glob_search |
| **Search Content** | GrepTool | (via shell) | grep | grep_search |
| **Web Fetch** | WebFetchTool | (MCP) | webfetch | web_fetch (built-in) |
| **Web Search** | WebSearchTool | (MCP) | websearch | web_search (built-in) |
| **Notebook** | NotebookEditTool | (MCP) | — | notebook_edit |
| **LSP** | LSPTool | — | — | lsp_query |
| **Git/PR** | (via Bash) | (via shell) | commit_and_pr | cisco_git (with Webex notify) |
| **Agent** | AgentTool | spawn_agent | agent | agent (typed sub-agents) |
| **Plan** | EnterPlanMode | — | plan_create | plan_mode (DAG-based) |
| **Memory** | (CLAUDE.md) | — | memory | memory (SQLite + FTS5) |
| **MCP** | MCPTool | MCP handler | mcp tools | mcp_tools |
| **Tasks** | TaskCreate/Update | — | — | task_manager |
| **Skills** | SkillTool | skills | skills | skill_runner |
| **Cisco-specific** | — | — | — | webex, duo, ise, dnac, meraki |

---

## 4. Permission & Security Model Comparison

| Aspect | Claude Code | Codex | Astro | Cisco-Code Target |
|--------|-------------|-------|-------|-------------------|
| **Permission Modes** | 6 modes (default, acceptEdits, bypass, dontAsk, plan, auto) | 3 modes (Restricted, WorkspaceWrite, Full) | 3 levels (ALLOW, WARN, CONFIRM) | 4 modes (readonly, workspace, elevated, admin) |
| **Rule System** | alwaysAllow/alwaysDeny/alwaysAsk per tool | Policy-based per sandbox | Per-tool permission level | Hierarchical rules (user < project < admin) |
| **Sandboxing** | None (permission prompts only) | OS-native (Seatbelt/Bubblewrap/tokens) | Path validation + Docker stubs | OS-native (from Codex) + container |
| **Network Isolation** | None | Per-sandbox network policy | SSRF protection on webfetch | Network namespace + allowlist |
| **Guardian LLM** | None | Second LLM reviews dangerous ops | None | Guardian LLM (from Codex pattern) |
| **Hooks** | Pre/Post tool use, session events | Pre/Post tool use | 15 lifecycle events | Full lifecycle hooks |
| **Audit** | Telemetry + analytics | Session recording | RL trace collection | Enterprise audit log + Splunk |

---

## 5. Model/Provider Integration Comparison

| Aspect | Claude Code | Codex | Astro | Cisco-Code Target |
|--------|-------------|-------|-------|-------------------|
| **Native Providers** | Anthropic | OpenAI | Anthropic, OpenAI, Google, Bedrock | All of the above + Cisco AI |
| **OpenAI-Compat** | No | Ollama, LM Studio | 14 endpoints (DeepSeek, Groq, etc.) | Full OpenAI-compat |
| **Local Models** | No | Ollama, LM Studio | Ollama, vLLM, SGLang | Ollama, vLLM, SGLang |
| **Streaming** | SSE | WebSocket → SSE → REST | SSE | WebSocket → SSE (Codex pattern) |
| **Prompt Caching** | Anthropic cache_control | Automatic prefix caching | Provider-specific | Unified caching layer |
| **Model Routing** | Manual toggle | Single model | 5-tier per-task | 5-tier (from Astro) |
| **Cost Tracking** | Per-model usage | Per-turn usage | Per-step with tier reporting | Enterprise cost dashboard |

---

## 6. Key Architectural Insights for cisco-code

### From Claude Code — Take:
1. **Rich ToolUseContext** — Tools need session-wide context (file cache, app state, MCP clients)
2. **Generator-based streaming** — Yield events, don't collect
3. **Tool result with side effects** — `ToolResult` can include new messages and context modifiers
4. **Mature permission rule system** — Per-tool allow/deny/ask rules
5. **Compaction service** — Automatic history compression when context overflows
6. **CLAUDE.md/memory system** — Project-scoped persistent context

### From Codex — Take:
7. **OS-native sandboxing** — Seatbelt (macOS), Bubblewrap+Landlock (Linux), restricted tokens (Windows)
8. **Transport fallback** — WebSocket → SSE → REST
9. **Feature flag lifecycle** — Developmental → Experimental → Stable → Deprecated
10. **Agent depth/count limits** — max_depth=1, max_threads=6
11. **Structured code review** — P0-P3 findings with confidence scores
12. **JSONL session recording** — Append-friendly persistence

### From Astro-Assistant — Take:
13. **Two-level planning** — DAG decomposition for complex tasks, direct ReAct for simple
14. **5-tier model routing** — SMALL/MEDIUM/LARGE/TEAMMATE/FRONTIER per-task
15. **Provider adapter ABC** — Clean abstraction for 10+ LLM backends
16. **Composable policy engine** — Tree of And/Or/Not safety policies
17. **RL trajectory collection** — (state, action, observation, reward) per step
18. **24 built-in skills** — Reusable workflow recipes
19. **Prompt assembly pipeline** — Layered: system + context + runtime + history + tools

### From Claw-Code-Parity — Take:
20. **Generic ConversationRuntime<C, T>** — Parameterized over ApiClient + ToolExecutor traits
21. **Modular Cargo workspace** — Clean crate boundaries
22. **Config hierarchy** — User < project < local scope
23. **CLAUDE.md auto-discovery** — Max 4KB/file, 12KB total
24. **OAuth PKCE flow** — Enterprise-grade authentication

---

## 7. What cisco-code Adds (Cisco-Specific)

| Feature | Description |
|---------|-------------|
| **Webex Integration** | Send notifications, create spaces, share code review results |
| **Duo MFA** | Enterprise authentication with Cisco Duo |
| **ISE Policy** | Network access control policy integration |
| **DNA Center** | Network device management and automation |
| **Meraki Dashboard** | Cloud network management API |
| **SecureX/XDR** | Security operations and threat intelligence |
| **AppDynamics** | Application performance monitoring integration |
| **ThousandEyes** | Network intelligence and monitoring |
| **Cisco AI Cloud** | Internal model hosting and inference |
| **Enterprise SSO** | SAML/OIDC with Cisco identity provider |
| **Splunk Audit** | Enterprise audit logging to Splunk |
| **Compliance Guardrails** | PII detection, IP protection, export control |

---

## 8. Language Choice Rationale: Rust + Python

**Why Rust for the core runtime:**
- Memory safety without GC (critical for long-running agent sessions)
- Native performance for tool execution, file I/O, sandboxing
- Codex proves Rust works well for this domain (70+ production crates)
- Claw-Code-Parity provides a reference Rust implementation
- Cross-platform compilation (macOS, Linux, Windows)
- Strong typing catches integration errors at compile time

**Why Python for extensibility layer:**
- Provider adapters leverage existing SDKs (anthropic, openai, google-genai, boto3)
- RL trajectory processing and model training
- Rapid prototyping of new tools and skills
- Rich ecosystem for code analysis (tree-sitter, AST manipulation)
- Astro-Assistant proves Python works for the intelligence layer

**Boundary:**
```
┌─────────────────────────────────────────────────────┐
│  Rust Core (performance-critical, safety-critical)  │
│  - Agent loop / state machine                       │
│  - Tool execution + sandboxing                      │
│  - Permission engine                                │
│  - Session persistence                              │
│  - TUI rendering                                    │
│  - MCP client/server                                │
│  - SSE/WebSocket streaming                          │
└───────────────────┬─────────────────────────────────┘
                    │ IPC (JSON-RPC over stdio/Unix socket)
┌───────────────────▼─────────────────────────────────┐
│  Python Extension Layer (intelligence, flexibility) │
│  - LLM provider adapters                            │
│  - Model tier routing                               │
│  - DAG planner / replanner                          │
│  - Custom tools and skills                          │
│  - RL trace collection                              │
│  - Code analysis (tree-sitter)                      │
│  - Cisco-specific integrations                      │
└─────────────────────────────────────────────────────┘
```

---

*This comparison document serves as the analytical foundation for cisco-code's architecture.
Each design decision traces back to a proven pattern from one of these four systems.*
