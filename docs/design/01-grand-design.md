# cisco-code: Grand Design Document

> A Cisco-branded AI coding agent built in **pure Rust**, synthesizing the best
> architectural ideas from Claude Code, Codex, Astro-Assistant, and Claw-Code-Parity.
> See [03-why-pure-rust.md](03-why-pure-rust.md) for the language decision analysis.

---

## 1. Vision

**cisco-code** is an enterprise-grade AI coding agent for Cisco engineers. It combines:

- **Claude Code's** mature tool orchestration and permission system
- **Codex's** OS-native sandboxing and Rust performance
- **Astro-Assistant's** DAG planning, multi-model routing, and RL readiness
- **Claw-Code-Parity's** clean Rust trait-based architecture

The result: a multi-model, security-hardened, plan-aware coding agent with Cisco-specific
integrations for network engineering, security operations, and collaboration.

---

## 2. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                         cisco-code CLI / TUI                        │
│                    (Ratatui terminal interface)                      │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌──────────────┐  ┌──────────────┐  ┌───────────────────────────┐ │
│  │  REPL Engine  │  │  Print Mode  │  │  App Server (HTTP/WS)    │ │
│  │  (interactive)│  │  (scripting) │  │  (IDE/web integration)   │ │
│  └──────┬───────┘  └──────┬───────┘  └───────────┬───────────────┘ │
│         └──────────────────┴──────────────────────┘                 │
│                            │                                        │
│  ┌─────────────────────────▼─────────────────────────────────────┐ │
│  │              ConversationRuntime<P, T>                         │ │
│  │  Generic over Provider (P) and ToolExecutor (T)               │ │
│  │                                                               │ │
│  │  ┌─────────────┐  ┌──────────────┐  ┌─────────────────────┐  │ │
│  │  │ System      │  │ Context      │  │ Message History     │  │ │
│  │  │ Prompt      │  │ Manager      │  │ + Compaction        │  │ │
│  │  │ Builder     │  │ (token budg.)│  │ + JSONL Persistence │  │ │
│  │  └─────────────┘  └──────────────┘  └─────────────────────┘  │ │
│  └───────────────────────────┬───────────────────────────────────┘ │
│                              │                                      │
│  ┌───────────────────────────▼───────────────────────────────────┐ │
│  │                    Task Router                                │ │
│  │  Complexity classifier → tier assignment → provider selection │ │
│  │                                                               │ │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌─────────────┐  │ │
│  │  │  SMALL   │  │  MEDIUM  │  │  LARGE   │  │  FRONTIER   │  │ │
│  │  │ (titles, │  │ (compact,│  │ (agent,  │  │ (plan,      │  │ │
│  │  │  class.) │  │  review) │  │  execute) │  │  research)  │  │ │
│  │  └──────────┘  └──────────┘  └──────────┘  └─────────────┘  │ │
│  └───────────────────────────┬───────────────────────────────────┘ │
│                              │                                      │
│  ┌───────────────────────────▼───────────────────────────────────┐ │
│  │                 Planning Engine (Complex Tasks)                │ │
│  │  ┌──────────┐  ┌──────────────┐  ┌──────────┐  ┌──────────┐ │ │
│  │  │ DAG      │  │ Parallel     │  │ Reflect  │  │ Replan   │ │ │
│  │  │ Builder  │  │ Executor     │  │ + Assess │  │ Engine   │ │ │
│  │  └──────────┘  └──────────────┘  └──────────┘  └──────────┘ │ │
│  └───────────────────────────────────────────────────────────────┘ │
│                                                                     │
├─────────────────────────────────────────────────────────────────────┤
│                         TOOL LAYER                                  │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  GlobalToolRegistry                                         │   │
│  │                                                             │   │
│  │  Built-in:  bash, read, write, edit, glob, grep,           │   │
│  │             web_fetch, web_search, agent, plan,             │   │
│  │             memory, notebook, lsp, skill, task              │   │
│  │                                                             │   │
│  │  Cisco:     webex, duo, ise, dnac, meraki, securex,        │   │
│  │             appd, thousandeyes, cisco_ai                    │   │
│  │                                                             │   │
│  │  External:  MCP tools (auto-discovered)                    │   │
│  │             Plugin tools (user-installed)                   │   │
│  └────────────────────────────┬────────────────────────────────┘   │
│                               │                                     │
│  ┌────────────────────────────▼────────────────────────────────┐   │
│  │  Permission Engine                                          │   │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌───────────┐  │   │
│  │  │ Rules    │  │ Guardian │  │ Hooks    │  │ Audit     │  │   │
│  │  │ (allow/  │  │ LLM      │  │ (pre/   │  │ Logger    │  │   │
│  │  │  deny)   │  │ (safety) │  │  post)   │  │ (Splunk)  │  │   │
│  │  └──────────┘  └──────────┘  └──────────┘  └───────────┘  │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Sandbox Engine                                             │   │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌───────────┐  │   │
│  │  │ Seatbelt │  │ Bubble-  │  │ Windows  │  │ Container │  │   │
│  │  │ (macOS)  │  │ wrap+    │  │ Tokens   │  │ (Docker)  │  │   │
│  │  │          │  │ Landlock │  │          │  │           │  │   │
│  │  └──────────┘  └──────────┘  └──────────┘  └───────────┘  │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
├─────────────────────────────────────────────────────────────────────┤
│                       PROVIDER LAYER                                │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  Provider Registry (Pure Rust, raw HTTP via reqwest)        │   │
│  │                                                             │   │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌───────────┐  │   │
│  │  │Anthropic │  │ OpenAI   │  │ Google   │  │ Bedrock   │  │   │
│  │  │ (Claude) │  │ (GPT)    │  │ (Gemini) │  │ (Multi)   │  │   │
│  │  └──────────┘  └──────────┘  └──────────┘  └───────────┘  │   │
│  │                                                             │   │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌───────────┐  │   │
│  │  │ Cisco AI │  │ Ollama   │  │ vLLM /   │  │ OpenAI-   │  │   │
│  │  │ Cloud    │  │ (local)  │  │ SGLang   │  │ Compat    │  │   │
│  │  └──────────┘  └──────────┘  └──────────┘  └───────────┘  │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
├─────────────────────────────────────────────────────────────────────┤
│                     PERSISTENCE LAYER                               │
│                                                                     │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────────┐   │
│  │ Sessions │  │ Memory   │  │ RL Traces│  │ Config           │   │
│  │ (JSONL)  │  │ (SQLite  │  │ (SQLite) │  │ (TOML/YAML,     │   │
│  │          │  │  + FTS5) │  │          │  │  hierarchical)   │   │
│  └──────────┘  └──────────┘  └──────────┘  └──────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 3. Core Design Principles

### 3.1 Principle 1: Tool-Centric Architecture (from Claude Code)

Everything revolves around tools. The LLM is a tool-calling engine; the runtime is a
tool-execution engine. Tools are first-class:

```rust
/// Core tool trait (Rust side)
#[async_trait]
pub trait Tool: Send + Sync {
    /// Tool identity
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> JsonSchema;

    /// Execution
    async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult>;

    /// Metadata for permission engine
    fn is_read_only(&self, input: &Value) -> bool;
    fn is_destructive(&self, input: &Value) -> bool;
    fn required_permission(&self) -> PermissionLevel;

    /// Concurrency safety
    fn is_concurrency_safe(&self, input: &Value) -> bool { false }
}
```

```rust
/// Provider trait — LLM backends implement this (all in Rust, raw HTTP)
#[async_trait]
pub trait Provider: Send + Sync {
    async fn stream(&self, request: CompletionRequest)
        -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent>> + Send>>>;
    fn model_info(&self, model_id: &str) -> ModelInfo;
    fn available_models(&self) -> Vec<String>;
}
```

No Python SDKs needed — LLM APIs are just HTTP+SSE. Codex and Claw-Code-Parity
both prove this works with `reqwest` + `serde_json`.

### 3.2 Principle 2: Security by Default (from Codex)

Every tool execution passes through the sandbox:

```
Tool.call() → PermissionEngine.check() → Sandbox.transform() → OS.execute()
```

The sandbox is **not optional**. Read-only tools skip sandboxing; write/execute tools
always go through the platform-specific sandbox transform.

### 3.3 Principle 3: Right Model for the Job (from Astro-Assistant)

Not every LLM call needs the biggest model:

| Tier | Use Case | Example Models |
|------|----------|---------------|
| SMALL | Title generation, classification | Haiku, GPT-4o-mini |
| MEDIUM | Compaction, code review | Sonnet, GPT-4o |
| LARGE | Main agent, execution | Opus, GPT-5 |
| TEAMMATE | Peer review, second opinion | Cross-vendor model |
| FRONTIER | Complex planning, research | Best available |

### 3.4 Principle 4: Plan When Needed, React When Sufficient (from Astro)

```rust
enum ExecutionStrategy {
    /// Simple tasks: direct ReAct loop
    Direct { model_tier: Tier },
    /// Complex tasks: DAG planning + parallel execution
    Planned { dag: TaskDAG, tiers: HashMap<TaskId, Tier> },
}
```

A lightweight classifier determines complexity. Simple tasks go direct; complex tasks
get a plan with parallel execution waves.

### 3.5 Principle 5: Generic Runtime (from Claw-Code-Parity)

```rust
pub struct ConversationRuntime<P: Provider, T: ToolExecutor> {
    provider: P,
    tools: T,
    session: Session,
    permissions: PermissionEngine,
    hooks: HookRunner,
    config: RuntimeConfig,
}
```

The runtime is generic over provider and tool executor. This enables:
- Unit testing with mock providers
- Swapping LLM backends at runtime
- Custom tool executors for different environments

---

## 4. Rust Crate Architecture

```
cisco-code/
├── Cargo.toml                 # Workspace root
├── crates/
│   ├── cli/                   # Binary: REPL, args, TUI rendering
│   │   └── src/
│   │       ├── main.rs        # Entry point + subcommand routing
│   │       ├── repl.rs        # Interactive REPL loop
│   │       ├── render.rs      # Terminal output (Ratatui)
│   │       ├── args.rs        # CLI argument parsing (clap)
│   │       └── app.rs         # App state orchestration
│   │
│   ├── runtime/               # Core: agent loop, session, permissions
│   │   └── src/
│   │       ├── lib.rs         # ConversationRuntime<P, T>
│   │       ├── conversation.rs # Turn execution + streaming
│   │       ├── session.rs     # JSONL persistence + compaction
│   │       ├── permissions.rs # Permission engine + rules
│   │       ├── hooks.rs       # Hook execution pipeline
│   │       ├── config.rs      # Hierarchical config loader
│   │       ├── prompt.rs      # System prompt builder
│   │       ├── compact.rs     # History compaction
│   │       └── context.rs     # Context window management
│   │
│   ├── tools/                 # Tool registry + built-in tools
│   │   └── src/
│   │       ├── lib.rs         # GlobalToolRegistry
│   │       ├── registry.rs    # Registration + lookup
│   │       ├── bash.rs        # Shell execution
│   │       ├── file_ops.rs    # read, write, edit, glob, grep
│   │       ├── web.rs         # web_fetch, web_search
│   │       ├── agent.rs       # Sub-agent spawning
│   │       ├── plan.rs        # Plan mode tools
│   │       ├── memory.rs      # Memory read/write
│   │       └── cisco/         # Cisco-specific tools
│   │           ├── mod.rs
│   │           ├── webex.rs
│   │           ├── duo.rs
│   │           ├── dnac.rs
│   │           ├── meraki.rs
│   │           └── securex.rs
│   │
│   ├── sandbox/               # OS-native sandboxing (from Codex)
│   │   └── src/
│   │       ├── lib.rs         # Sandbox trait + transform pipeline
│   │       ├── seatbelt.rs    # macOS Seatbelt profiles
│   │       ├── bubblewrap.rs  # Linux Bubblewrap + Landlock
│   │       ├── windows.rs     # Windows restricted tokens
│   │       └── docker.rs      # Container-based sandbox
│   │
│   ├── api/                   # HTTP client + SSE streaming
│   │   └── src/
│   │       ├── lib.rs         # Client trait
│   │       ├── sse.rs         # SSE parser
│   │       ├── websocket.rs   # WebSocket transport
│   │       └── types.rs       # API request/response types
│   │
│   ├── mcp/                   # Model Context Protocol
│   │   └── src/
│   │       ├── lib.rs         # MCP client + server
│   │       ├── transport.rs   # stdio/SSE/WebSocket transport
│   │       ├── discovery.rs   # Auto-discovery
│   │       └── bridge.rs      # MCP tool → cisco-code tool bridge
│   │
│   ├── protocol/              # Shared types + event models
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── messages.rs    # Message types (user, assistant, system, tool)
│   │       ├── events.rs      # Stream events
│   │       ├── tools.rs       # Tool schemas + results
│   │       └── errors.rs      # Error types
│   │
│   └── telemetry/             # Observability + RL traces
│       └── src/
│           ├── lib.rs
│           ├── traces.rs      # RL trajectory collection
│           ├── metrics.rs     # Usage + cost tracking
│           └── audit.rs       # Enterprise audit logging
│
│   ├── providers/             # LLM provider adapters (pure Rust HTTP)
│   │   └── src/
│   │       ├── lib.rs         # Provider trait + registry
│   │       ├── anthropic.rs   # Anthropic (reqwest + SSE)
│   │       ├── openai.rs      # OpenAI (reqwest + SSE)
│   │       ├── google.rs      # Google Gemini
│   │       ├── bedrock.rs     # AWS Bedrock
│   │       ├── cisco_ai.rs    # Cisco internal AI cloud
│   │       ├── openai_compat.rs # Any OpenAI-compatible endpoint
│   │       ├── cost.rs        # Token cost estimation
│   │       └── routing.rs     # 5-tier model routing
│   │
│   └── planning/              # DAG planner + executor (pure Rust)
│       └── src/
│           ├── lib.rs
│           ├── classifier.rs  # Complexity classifier
│           ├── dag.rs         # TaskDAG (petgraph)
│           ├── planner.rs     # LLM-based plan generation
│           ├── executor.rs    # Parallel wave executor
│           └── replanner.rs   # Adaptive replanning
│
├── scripts/                   # Offline analysis tools (Python, optional)
│   ├── rl_export.py           # Export RL traces for training
│   ├── eval_swebench.py       # SWE-bench evaluation
│   └── cost_report.py         # Cost analysis from traces
│
├── config/                    # Default configuration
│   ├── default.toml           # Default settings
│   ├── model_tiers.toml       # Model tier definitions
│   └── cisco.toml             # Cisco-specific defaults
│
├── skills/                    # Built-in skill definitions
│   ├── code-review/
│   ├── commit-and-pr/
│   ├── network-config/        # Cisco: network device configuration
│   ├── security-audit/        # Cisco: security posture review
│   └── incident-response/     # Cisco: SecureX incident workflows
│
└── docs/                      # Design documentation
    ├── design/
    └── comparisons/
```

---

## 5. Data Flow: End-to-End Turn (Pure Rust)

```
1. User types prompt in TUI
     │
2. CLI parses input, detects slash commands
     │
3. ConversationRuntime receives user message
     │
4. Prompt builder assembles: system + context + history + tools
     │
5. Router classifies complexity → assigns tier → selects provider
     │
6. Provider streams LLM response via SSE/WebSocket (reqwest)
     │
7. Runtime parses stream events
     │  ├─ Text block → render to TUI
     │  └─ Tool_use block → enter tool pipeline
     │
8. Tool pipeline:
     │  a. PermissionEngine.check(tool, input, rules)
     │  b. HookRunner.pre_tool_use(tool, input)
     │  c. Sandbox.transform(tool, input) → sandboxed command
     │  d. Tool.call(sandboxed_input, context)
     │  e. HookRunner.post_tool_use(tool, input, result)
     │  f. TraceCollector.record(state, action, observation)
     │
9. Tool results sent back to LLM
     │
10. Loop until stop_reason = "end_turn" or budget exhausted
     │
11. Final response rendered, session persisted (JSONL)

All steps are pure Rust. No Python process, no IPC, no subprocess calls.
LLM APIs are HTTP — reqwest handles streaming SSE natively.
```

---

## 7. Configuration System

Hierarchical config with TOML format (cleaner than JSON for humans):

```
Precedence (later wins):
  1. config/default.toml          (built-in defaults)
  2. ~/.cisco-code/config.toml    (user global)
  3. .cisco-code/config.toml      (project)
  4. .cisco-code/config.local.toml (machine-local, gitignored)
  5. Environment variables (CISCO_CODE_*)
  6. CLI flags
```

```toml
# Example config/default.toml

[general]
default_provider = "anthropic"
default_model = "claude-sonnet-4-6"
plan_mode = "auto"              # off | ask | auto | always
permission_mode = "default"      # readonly | default | elevated | admin

[model_tiers]
small = "anthropic/claude-haiku-4-5"
medium = "anthropic/claude-sonnet-4-6"
large = "anthropic/claude-opus-4-6"
teammate = "openai/gpt-5"
frontier = "anthropic/claude-opus-4-6"

[role_tiers]
main_agent = "large"
planner = "large"
executor = "large"
reviewer = "teammate"
compaction = "medium"
classifier = "small"
title = "small"

[sandbox]
mode = "os-native"               # os-native | container | none
network = "workspace-only"       # none | workspace-only | allowlist | full
allowed_hosts = ["github.com", "pypi.org", "crates.io"]

[cisco]
webex_bot_token = ""             # Or from CISCO_CODE_WEBEX_TOKEN
duo_integration_key = ""
sso_provider = "okta"
audit_endpoint = ""              # Splunk HEC endpoint

[hooks]
pre_tool_use = []
post_tool_use = []
session_start = []
session_end = []

[mcp.servers]
# External MCP servers
```

---

## 8. Cisco-Specific Features

### 8.1 Webex Integration Tool
```
cisco_webex:
  - Send code review summaries to Webex spaces
  - Create incident spaces with relevant engineers
  - Share agent session transcripts
  - Notify on PR merge/CI failure
```

### 8.2 Network Engineering Tools
```
cisco_dnac:
  - Query DNA Center for device inventory
  - Push configuration templates
  - Run compliance checks

cisco_meraki:
  - Dashboard API for network status
  - Deploy configuration changes
  - Monitor alerts

cisco_ise:
  - Query ISE policies
  - Check endpoint compliance
  - Manage network access rules
```

### 8.3 Security Operations Tools
```
cisco_securex:
  - Query threat intelligence
  - Correlate observables (IPs, domains, hashes)
  - Trigger response playbooks

cisco_xdr:
  - Incident investigation
  - Automated triage
  - Evidence collection
```

### 8.4 Enterprise Compliance
```
cisco_compliance:
  - PII detection in code (before commit)
  - IP/trade secret protection
  - Export control classification
  - License compatibility checking
  - Secrets scanning (API keys, certificates)
```

### 8.5 Authentication
```
cisco_auth:
  - SAML/OIDC SSO with Cisco identity provider
  - Duo MFA for elevated permissions
  - API key management with Vault
  - Certificate-based auth for internal services
```

---

## 9. Comparison with Existing Systems

| Feature | Claude Code | Codex | Astro | **cisco-code** |
|---------|-------------|-------|-------|----------------|
| Language | TypeScript | Rust | Python+TS | **Rust+Python** |
| Agent loop | Flat ReAct | Flat ReAct | Two-level DAG | **Two-level DAG** |
| Providers | 1 (Anthropic) | 1 (OpenAI) | 10+ | **10+ (incl Cisco AI)** |
| Model routing | None | None | 5-tier | **5-tier** |
| Sandboxing | None | OS-native | Stubs | **OS-native + container** |
| Guardian LLM | None | Yes | None | **Yes** |
| Planning | None | None | DAG | **DAG with replan** |
| RL traces | None | None | Yes | **Yes** |
| Enterprise SSO | None | None | OAuth | **SAML/OIDC + Duo** |
| Network tools | None | None | None | **DNAC, Meraki, ISE** |
| Security tools | None | None | None | **SecureX, XDR** |
| Collaboration | None | None | None | **Webex integration** |
| Audit logging | Analytics | Recording | Traces | **Splunk + traces** |
| Compliance | None | None | None | **PII/IP/export control** |

---

## 10. Design Insights for the Community

This section documents architectural patterns that are valuable beyond cisco-code:

### 10.1 The Tool-as-Universe Pattern (Claude Code)
In Claude Code, tools are not just "functions the LLM can call." They are the primary
unit of composition. A tool result can inject new messages, modify context, spawn sub-agents,
and reshape the entire conversation. This is more powerful than the typical
`tool(input) → output` pattern.

### 10.2 The Sandbox Transform Pipeline (Codex)
Codex's insight: don't sandbox the tool — sandbox the *execution*. The `CommandSpec →
SandboxTransformRequest → ExecRequest` pipeline means the tool author writes normal code,
and the sandbox layer wraps it transparently. This separation of concerns is cleaner than
having each tool implement its own safety checks.

### 10.3 The Two-Level Agent (Astro-Assistant)
The key insight: flat ReAct loops work for simple tasks but degrade on complex multi-step
problems. A planning layer that decomposes into a DAG enables parallel execution, model
tier optimization, and adaptive replanning. The complexity classifier is the gate between
the two levels.

### 10.4 The Generic Runtime (Claw-Code-Parity)
`ConversationRuntime<C: ApiClient, T: ToolExecutor>` is elegant because it makes the
runtime testable and composable. Mock the API client for unit tests. Swap the tool executor
for a sandboxed variant. The trait bounds document the contract.

### 10.5 The IPC Boundary (cisco-code original)
Putting intelligence (LLM calls, planning, routing) in Python and execution (tools,
sandbox, permissions) in Rust creates a natural boundary. Python handles the parts that
change fast (new models, new providers, new planning strategies). Rust handles the parts
that must be fast and safe (file I/O, process spawning, permission checks).

---

*This document is the north star for cisco-code development. All implementation decisions
should trace back to the principles and patterns described here.*
