# cisco-code: Step-by-Step Implementation Roadmap

> Phased development plan. Each phase produces a working system with increasing capability.
> Every phase includes design documentation explaining the architectural insights.

---

## Phase Overview

```
Phase 0: Foundation (Week 1-2)
  → Rust workspace + Python package + basic REPL
  → Design doc: "Why Rust+Python and how they communicate"

Phase 1: Core Agent Loop (Week 3-4)
  → ConversationRuntime with single provider (Anthropic)
  → Design doc: "Anatomy of an agentic turn"

Phase 2: Tool System (Week 5-7)
  → 10 built-in tools (bash, read, write, edit, glob, grep, web, agent, memory, plan)
  → Design doc: "Tool-centric architecture patterns"

Phase 3: Multi-Model Support (Week 8-9)
  → Provider registry, model routing, 5-tier system
  → Design doc: "Multi-model routing and cost optimization"

Phase 4: Permission & Security (Week 10-11)
  → Permission engine, hooks, OS-native sandboxing
  → Design doc: "Security architecture for AI agents"

Phase 5: Planning Engine (Week 12-14)
  → DAG planner, parallel executor, replanner
  → Design doc: "Hierarchical planning vs flat ReAct"

Phase 6: Cisco Integration (Week 15-17)
  → Webex, DNAC, Meraki, SecureX, SSO, compliance
  → Design doc: "Enterprise AI agent integration patterns"

Phase 7: MCP & Extensibility (Week 18-19)
  → MCP client/server, plugins, skills
  → Design doc: "Open protocols for agent extensibility"

Phase 8: RL & Observability (Week 20-21)
  → Trajectory collection, export, cost dashboard
  → Design doc: "RL-ready agent design"

Phase 9: Polish & Production (Week 22-24)
  → TUI enhancements, IDE integration, documentation
  → Design doc: "Production deployment patterns"
```

---

## Phase 0: Foundation

**Goal:** Establish the Rust+Python project structure with IPC, basic CLI, and build system.

### Tasks

- [ ] **0.1** Create Rust workspace with crate structure
  ```
  crates/: cli, runtime, tools, sandbox, api, mcp, protocol, telemetry
  ```

- [ ] **0.2** Create Python package structure
  ```
  python/cisco_code/: providers, planning, routing, skills, rl, cisco
  ```

- [ ] **0.3** Implement JSON-RPC IPC layer
  - Rust side: `ipc` module in runtime crate
  - Python side: `cisco_code.ipc` module
  - Unix domain socket transport
  - Basic request/response + streaming

- [ ] **0.4** Implement basic CLI (clap)
  - `cisco-code` — interactive REPL stub
  - `cisco-code prompt "text"` — one-shot mode
  - `cisco-code login` — API key setup
  - `cisco-code doctor` — environment check

- [ ] **0.5** Implement config loader
  - TOML parsing with serde
  - Hierarchical merge (default < user < project < local < env < cli)
  - Create `config/default.toml`

- [ ] **0.6** Write design doc: `03-rust-python-architecture.md`
  - Why Rust for core, Python for intelligence
  - IPC design rationale (JSON-RPC vs PyO3 vs gRPC)
  - Comparison with how Codex (pure Rust) and Astro (pure Python) handle this
  - Process lifecycle management

### Deliverable
A `cisco-code doctor` command that starts, validates config, tests IPC to Python, and exits.

---

## Phase 1: Core Agent Loop

**Goal:** Single-turn conversation with Anthropic, streaming response to terminal.

### Tasks

- [ ] **1.1** Define message types in `protocol` crate
  ```rust
  enum Message { User, Assistant, System, ToolUse, ToolResult }
  enum ContentBlock { Text, ToolUse, ToolResult, Image }
  ```

- [ ] **1.2** Implement SSE streaming client in `api` crate
  - Parse Anthropic SSE format
  - Handle `message_start`, `content_block_delta`, `message_stop`
  - Token usage tracking

- [ ] **1.3** Implement `ConversationRuntime<P, T>` in `runtime` crate
  - Generic over Provider and ToolExecutor traits
  - Turn execution: prompt → API call → parse response → yield events
  - Basic system prompt builder (hardcoded for now)

- [ ] **1.4** Implement Anthropic provider adapter (Python)
  - Use `anthropic` SDK
  - Streaming completions
  - Tool schema formatting

- [ ] **1.5** Connect CLI REPL to runtime
  - User input → runtime → streaming output to terminal
  - Basic markdown rendering

- [ ] **1.6** Implement session persistence
  - JSONL format (append per turn)
  - Session ID management
  - `~/.cisco-code/sessions/` directory

- [ ] **1.7** Write design doc: `04-agentic-turn-anatomy.md`
  - The agentic loop pattern across Claude Code, Codex, Astro
  - Streaming architecture (generator vs callback vs channel)
  - How ConversationRuntime<P, T> generalizes the pattern
  - Session persistence strategies (JSONL vs SQLite vs hybrid)

### Deliverable
Interactive chat with Claude via terminal, responses streamed in real-time, sessions saved.

---

## Phase 2: Tool System

**Goal:** 10 built-in tools with permission checks, making the agent capable of real coding tasks.

### Tasks

- [ ] **2.1** Define `Tool` trait and `ToolResult` type
  ```rust
  trait Tool: Send + Sync {
      fn name(&self) -> &str;
      fn input_schema(&self) -> JsonSchema;
      async fn call(&self, input: Value, ctx: &ToolContext) -> Result<ToolResult>;
      fn is_read_only(&self, input: &Value) -> bool;
  }
  ```

- [ ] **2.2** Implement `GlobalToolRegistry`
  - Registration by name
  - Schema generation for API (tool definitions sent to LLM)
  - Lookup by name

- [ ] **2.3** Implement file tools: `read_file`, `write_file`, `edit_file`
  - `read_file`: Line-numbered output, offset/limit support
  - `write_file`: Create/overwrite with safety checks
  - `edit_file`: String replacement (Claude Code pattern) — unique match required

- [ ] **2.4** Implement search tools: `glob_search`, `grep_search`
  - `glob_search`: Fast file pattern matching
  - `grep_search`: ripgrep-based content search with regex

- [ ] **2.5** Implement `bash` tool
  - Process spawning with timeout
  - stdout/stderr capture
  - Working directory management
  - Basic command blocklist

- [ ] **2.6** Implement `web_fetch` tool
  - URL fetching with markdown extraction
  - SSRF protection (private IP blocking)
  - Size limits

- [ ] **2.7** Implement `agent` tool (sub-agent spawning)
  - Spawn child ConversationRuntime
  - Scoped context (isolated state)
  - Depth limiting (max 2 levels)
  - Background execution option

- [ ] **2.8** Implement `memory` tool
  - SQLite + FTS5 for persistent memory
  - Read/write/search operations
  - Scoped to project

- [ ] **2.9** Wire tools into ConversationRuntime
  - Tool definitions sent to LLM in API request
  - Tool use blocks parsed and routed to handlers
  - Tool results sent back as next turn
  - Multi-turn loop until end_turn

- [ ] **2.10** Write design doc: `05-tool-centric-architecture.md`
  - How Claude Code makes tools first-class (ToolUseContext pattern)
  - How Codex uses registry + handler trait
  - How Astro uses decorator-based registration
  - The tool result as side-effect channel (Claude Code's newMessages pattern)
  - Tool concurrency safety analysis

### Deliverable
Agent that can read/write/edit files, search code, run commands, browse web, and spawn sub-agents.

---

## Phase 3: Multi-Model Support

**Goal:** Support 10+ LLM providers with intelligent tier-based routing.

### Tasks

- [ ] **3.1** Implement provider adapter ABC (Python)
  ```python
  class ProviderAdapter(ABC):
      async def complete(self, request) -> CompletionResult
      async def stream(self, request) -> AsyncIterator[StreamChunk]
      def model_info(self, model_id) -> ModelInfo
  ```

- [ ] **3.2** Implement provider adapters
  - Anthropic (Claude family)
  - OpenAI (GPT family)
  - Google (Gemini family)
  - AWS Bedrock (multi-vendor)
  - OpenAI-compatible (DeepSeek, Groq, Ollama, vLLM, etc.)
  - Cisco AI Cloud (internal)

- [ ] **3.3** Implement provider registry
  - Auto-detection from API keys
  - Lazy initialization
  - Health checks

- [ ] **3.4** Implement model tier routing
  - Tier definitions in config (SMALL/MEDIUM/LARGE/TEAMMATE/FRONTIER)
  - Role → tier mapping
  - Cost tracking per tier

- [ ] **3.5** Implement complexity classifier
  - Lightweight LLM call (SMALL tier) to classify task
  - Determines: simple vs complex, recommended tier, tool requirements

- [ ] **3.6** Implement transport fallback (Rust api crate)
  - WebSocket (preferred) → SSE (fallback) → REST (final)
  - Per-provider transport detection

- [ ] **3.7** Write design doc: `06-multi-model-routing.md`
  - Why Astro's 5-tier system saves 80% cost
  - Provider adapter pattern comparison (Astro vs Codex vs Claude Code)
  - Transport fallback strategy (from Codex)
  - Prompt caching across providers
  - Cost estimation and tracking

### Deliverable
`cisco-code --model openai/gpt-5 "explain this code"` works. Automatic tier routing in effect.

---

## Phase 4: Permission & Security

**Goal:** Enterprise-grade security with OS-native sandboxing and permission engine.

### Tasks

- [ ] **4.1** Implement permission engine (Rust)
  - Rule evaluation: allow → deny → ask chain
  - Per-tool permission levels
  - Config-driven rules

- [ ] **4.2** Implement hook system
  - Pre/post tool use hooks
  - Session start/end hooks
  - Hook configuration in TOML
  - Shell command execution for hooks

- [ ] **4.3** Implement macOS Seatbelt sandbox
  - Profile generation per tool
  - File system restrictions
  - Network isolation

- [ ] **4.4** Implement Linux Bubblewrap + Landlock sandbox
  - User namespace isolation
  - LSM file access control
  - Network namespace

- [ ] **4.5** Implement Guardian LLM (from Codex pattern)
  - Second LLM call reviews dangerous operations
  - Risk scoring (0-100)
  - Block threshold configurable
  - Uses SMALL tier model (cost-effective)

- [ ] **4.6** Implement audit logging
  - All tool executions logged
  - Permission decisions logged
  - Structured format for Splunk/SIEM

- [ ] **4.7** Write design doc: `07-security-architecture.md`
  - Codex's OS-native sandbox architecture deep-dive
  - Permission model comparison (Claude Code 6 modes vs Codex 3 vs Astro 3)
  - Guardian LLM pattern: treating agent context as untrusted
  - The hook pipeline as extensible policy enforcement
  - Enterprise audit requirements

### Deliverable
`cisco-code --sandbox os-native` runs tools in Seatbelt/Bubblewrap. Permission prompts work.

---

## Phase 5: Planning Engine

**Goal:** DAG-based task decomposition for complex multi-step tasks.

### Tasks

- [ ] **5.1** Implement TaskDAG data structure (Python)
  - Nodes: tasks with dependencies
  - Edges: dependency relationships
  - Topological sorting for execution waves

- [ ] **5.2** Implement planner
  - LLM generates structured plan (JSON schema)
  - Plan → DAG conversion
  - Per-task tier assignment

- [ ] **5.3** Implement parallel wave executor
  - Independent tasks in same wave run concurrently
  - Each task = focused ReAct loop with scoped context
  - Results aggregation

- [ ] **5.4** Implement replanner
  - Milestone reflection (after each wave)
  - Failure detection and diagnosis
  - Adaptive replanning: retry, escalate tier, decompose further

- [ ] **5.5** Implement plan visualization
  - ASCII DAG in terminal
  - Task status tracking (pending/running/done/failed)
  - Progress percentage

- [ ] **5.6** Wire planning into runtime
  - Complexity classifier gates plan mode
  - Plan mode entry/exit
  - User can force plan mode with flag

- [ ] **5.7** Write design doc: `08-hierarchical-planning.md`
  - Why flat ReAct degrades on complex tasks (evidence from Astro)
  - DAG decomposition vs linear step lists
  - Parallel execution and cost optimization
  - Replanning as first-class operation (not failure recovery)
  - Comparison: Claude Code's plan mode vs Astro's DAG planner vs Codex (no planning)

### Deliverable
`cisco-code --plan "refactor the auth module"` produces a DAG, executes waves in parallel.

---

## Phase 6: Cisco Integration

**Goal:** Cisco-specific tools and enterprise features.

### Tasks

- [ ] **6.1** Implement Webex tool
  - Send messages to spaces
  - Create spaces
  - Share code/transcripts
  - Webhook notifications

- [ ] **6.2** Implement DNAC tool
  - Device inventory queries
  - Configuration templates
  - Compliance checks

- [ ] **6.3** Implement Meraki tool
  - Dashboard API integration
  - Network status queries
  - Configuration deployment

- [ ] **6.4** Implement SecureX/XDR tool
  - Threat intelligence queries
  - Incident investigation
  - Response playbook triggers

- [ ] **6.5** Implement Cisco SSO
  - SAML/OIDC integration
  - Duo MFA for elevated permissions
  - Token refresh management

- [ ] **6.6** Implement compliance guardrails
  - PII detection (pre-commit hook)
  - Secrets scanning
  - License compatibility
  - Export control classification

- [ ] **6.7** Implement network engineering skills
  - `network-config` skill: IOS-XE/NX-OS configuration
  - `security-audit` skill: security posture review
  - `incident-response` skill: SecureX playbooks

- [ ] **6.8** Write design doc: `09-enterprise-integration.md`
  - Enterprise AI agent integration patterns
  - Authentication flow (SSO → Duo → Vault)
  - Compliance-as-code for AI agents
  - Cisco API ecosystem integration

### Deliverable
Agent can send Webex messages, query DNA Center, trigger SecureX playbooks.

---

## Phase 7: MCP & Extensibility

**Goal:** Full MCP support and plugin/skill system for community extensibility.

### Tasks

- [ ] **7.1** Implement MCP client (Rust)
  - stdio transport
  - SSE transport
  - Tool discovery and registration
  - Elicitation handling

- [ ] **7.2** Implement MCP server (Rust)
  - Expose cisco-code tools via MCP
  - Enable other agents to use cisco-code as a tool provider

- [ ] **7.3** Implement plugin system
  - Plugin discovery (local + registry)
  - Plugin lifecycle (install, load, unload)
  - Plugin API (tools, hooks, commands)

- [ ] **7.4** Implement skill system
  - SKILL.md format support
  - Skill discovery (built-in < global < project)
  - Prompt injection for active skills
  - 10+ built-in skills

- [ ] **7.5** Implement slash commands
  - /help, /status, /config, /model, /plan
  - /commit, /pr, /review (git workflows)
  - /webex, /network (Cisco-specific)
  - Custom command registration

- [ ] **7.6** Write design doc: `10-extensibility-protocols.md`
  - MCP as the universal agent protocol
  - Plugin architectures compared (Claude Code vs Codex vs Astro)
  - Skill system design (SKILL.md format)
  - How open protocols enable ecosystem growth

### Deliverable
External MCP servers work. Users can write custom plugins and skills.

---

## Phase 8: RL & Observability

**Goal:** RL-ready trajectory collection and enterprise observability.

### Tasks

- [ ] **8.1** Implement trajectory collector (Python)
  - Record (state, action, observation, reward) per step
  - Structured SQLite storage

- [ ] **8.2** Implement trace exporter
  - JSON trajectory format
  - SWE-bench compatible format
  - Summary reports (tokens, cost, duration, success rate)

- [ ] **8.3** Implement cost dashboard
  - Per-model usage tracking
  - Per-tier cost breakdown
  - Per-project aggregation
  - Budget alerts

- [ ] **8.4** Implement enterprise telemetry
  - OpenTelemetry integration
  - Splunk HEC export
  - Custom metrics (tool usage, permission decisions, plan success rate)

- [ ] **8.5** Write design doc: `11-rl-ready-design.md`
  - Why RL-ready design matters for AI agents
  - Trajectory format and collection strategy
  - From traces to training data
  - Cost optimization through observability

### Deliverable
Every session produces RL trajectories. Cost dashboard shows per-model/per-project spend.

---

## Phase 9: Polish & Production

**Goal:** Production-ready release with beautiful TUI, comprehensive docs, and IDE integration.

### Tasks

- [ ] **9.1** TUI enhancements (Ratatui)
  - Syntax-highlighted code blocks
  - Inline diff rendering
  - Plan DAG visualization
  - Tool progress spinners
  - Split-pane for long outputs

- [ ] **9.2** IDE integration
  - VS Code extension (app-server protocol)
  - JetBrains plugin
  - Vim/Neovim integration

- [ ] **9.3** Session management
  - Resume, fork, search sessions
  - Session sharing (Webex integration)
  - Session export (markdown, HTML)

- [ ] **9.4** Comprehensive documentation
  - User guide
  - Developer guide (custom tools, plugins, skills)
  - Architecture guide (for this design doc series)
  - API reference

- [ ] **9.5** Testing & CI
  - Unit tests for all crates
  - Integration tests (mock LLM)
  - E2E tests
  - SWE-bench evaluation
  - CI pipeline (GitHub Actions)

- [ ] **9.6** Write design doc: `12-production-deployment.md`
  - Deployment patterns (standalone, server, container)
  - Scaling considerations
  - Monitoring and alerting
  - Incident response for AI agents

### Deliverable
Production-ready `cisco-code` with documentation, tests, and IDE integration.

---

## Phase Dependencies

```
Phase 0 (Foundation)
  └── Phase 1 (Agent Loop)
       ├── Phase 2 (Tools)
       │    ├── Phase 4 (Security) ←── depends on tools existing
       │    ├── Phase 6 (Cisco) ←── depends on tool system
       │    └── Phase 7 (MCP) ←── depends on tool system
       │
       └── Phase 3 (Multi-Model)
            └── Phase 5 (Planning) ←── depends on routing
                 └── Phase 8 (RL) ←── depends on planning traces
                      └── Phase 9 (Polish)
```

**Phases 4, 6, 7 can run in parallel** after Phase 2 completes.
**Phase 8** can start partially after Phase 2 (tool traces) but needs Phase 5 for plan traces.

---

## Quick Start: What to Build First

If you want the fastest path to a working demo:

1. **Phase 0.1-0.2**: Create project structure (1 day)
2. **Phase 1.2-1.3**: SSE client + runtime loop (2 days)
3. **Phase 1.4**: Anthropic provider (1 day)
4. **Phase 2.3-2.5**: File tools + bash (2 days)
5. **Phase 2.9**: Wire tools into runtime (1 day)

**= 7 days to a working AI coding agent that can read, write, and execute code.**

---

*This roadmap is designed for incremental delivery. Each phase produces a working system.
The design docs created at each phase serve as a knowledge base for understanding
agentic harness architecture.*
