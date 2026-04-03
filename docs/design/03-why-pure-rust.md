# Why Pure Rust: Language Decision Analysis

> This document explains why cisco-code is built entirely in Rust, with no Python
> runtime dependency. It compares the language choices of Claude Code (TypeScript),
> Codex (Rust), Astro-Assistant (Python), and Claw-Code-Parity (Rust).

---

## 1. What the Reference Systems Chose

| System | Language | Runtime Deps | Binary Size | Startup |
|--------|----------|-------------|-------------|---------|
| **Claude Code** | TypeScript | Node.js + npm (hundreds of packages) | ~100MB+ installed | ~2-3s |
| **Codex** | Rust | None | ~15MB single binary | <100ms |
| **Astro-Assistant** | Python + TS | Python 3.11 + pip + Node.js | ~200MB+ installed | ~1-2s |
| **Claw-Code-Parity** | Rust | None | ~10MB single binary | <100ms |
| **cisco-code** | **Rust** | **None** | **~15MB single binary** | **<100ms** |

Both production Rust systems (Codex, Claw) prove that **pure Rust is sufficient**
for a full-featured AI coding agent.

---

## 2. The Python Temptation — and Why It's Wrong

The initial design included Python for:
1. LLM provider SDKs (`anthropic`, `openai`, `google-genai`, `boto3`)
2. RL trajectory processing
3. DAG planning logic
4. Tree-sitter code analysis

**Why each doesn't actually need Python:**

### 2.1 LLM Provider APIs Are Just HTTP

Every LLM provider exposes a REST API with SSE streaming. The Python SDKs are thin
wrappers around HTTP calls. Rust's `reqwest` + `serde_json` does the same thing:

```rust
// Anthropic API call in Rust (from Claw-Code-Parity)
let response = self.client
    .post("https://api.anthropic.com/v1/messages")
    .header("x-api-key", &self.api_key)
    .header("anthropic-version", "2023-06-01")
    .json(&request)
    .send()
    .await?;
```

Codex supports OpenAI + Ollama + LM Studio — all in pure Rust HTTP.
Claw supports Anthropic + OpenAI-compatible — all in pure Rust HTTP.

**No Python SDK needed.**

### 2.2 RL Trajectories Are Just Structured Data

Writing `(state, action, observation, reward)` tuples to SQLite/JSONL is trivial
in Rust with `rusqlite` or `serde_json`. The analysis/training happens offline in
a separate Python environment, not in the agent runtime.

### 2.3 DAG Planning Is Just Graph Operations

A task DAG is a data structure (nodes + edges) with topological sorting. Rust's
`petgraph` crate handles this natively. The LLM generates the plan via an API call;
the Rust runtime executes it.

### 2.4 Tree-sitter Is Written in C with Rust Bindings

The `tree-sitter` crate provides first-class Rust bindings. No Python needed.

---

## 3. What Pure Rust Gives You

### 3.1 Single Binary Distribution
```bash
# Claude Code installation:
npm install -g @anthropic-ai/claude-code  # pulls 500+ packages

# Astro-Assistant installation:
pip install -e ".[all]"  # Python 3.11+ required, pip conflicts possible
cd cli && npm install && npm run build  # also needs Node.js

# cisco-code installation:
curl -L https://github.com/cisco/cisco-code/releases/latest/download/cisco-code-$(uname -s)-$(uname -m) -o cisco-code
chmod +x cisco-code
# Done. No runtime dependencies.
```

For enterprise deployment at Cisco (thousands of engineers), this is a massive win.
No "install Python 3.11, then pip install, then also Node.js" instructions.

### 3.2 Instant Startup
```
Claude Code:  ~2-3 seconds (Node.js cold start + module loading)
Astro:        ~1-2 seconds (Python interpreter + imports)
cisco-code:   <100ms (native binary, all code already in memory)
```

This matters for CI/CD integration and scripted usage where the agent is invoked
repeatedly.

### 3.3 Memory Safety Without GC
Long agent sessions (hundreds of tool calls, hours of interaction) suffer from GC
pauses in Node.js and Python. Rust's ownership model provides memory safety with
zero runtime overhead.

### 3.4 OS-Native Sandboxing
Rust's FFI makes it natural to call platform security APIs:
- macOS `sandbox_init()` for Seatbelt profiles
- Linux `prctl(PR_SET_NO_NEW_PRIVS)` + Landlock syscalls
- Windows `CreateRestrictedToken()`

Claude Code has **no sandboxing** — partly because Node.js makes FFI awkward.
Codex has the **best sandboxing** in the industry — because Rust makes FFI natural.

### 3.5 Fearless Concurrency
Parallel tool execution (independent tasks in a DAG wave) is safe and efficient
with Tokio's async runtime. No GIL (Python) or callback-hell (Node.js).

### 3.6 No Dependency Hell
```
Claude Code: node_modules/ with 500+ packages, npm audit warnings
Astro:       requirements.txt conflicts, virtualenv management
cisco-code:  Cargo.lock with exact versions, statically linked
```

---

## 4. What Pure Rust Costs You

| Trade-off | Impact | Mitigation |
|-----------|--------|------------|
| Slower iteration on provider adapters | Medium | HTTP APIs are stable; adapters change rarely |
| Longer compile times | Low | Incremental builds; only ~37K LOC in Claw |
| Smaller LLM/ML ecosystem | Low | Agent doesn't do ML; it calls APIs |
| Less accessible to contributors | Medium | Rust is increasingly common; good docs help |
| No REPL for experimentation | Low | Can expose a Python REPL as a tool (like Claw does) |

---

## 5. The Right Boundary for Python

Python is not banned — it's just not a **runtime dependency**:

- **Agent runtime:** Pure Rust (CLI, tools, sandbox, permissions, session, streaming)
- **Offline analysis:** Python scripts for RL trajectory processing, evaluation, benchmarking
- **User tool:** Python REPL available as a tool for users to execute their own Python code
- **MCP servers:** External MCP tool servers can be written in any language (Python, TS, Go)
- **Cisco integrations:** Can be Rust crates OR external MCP servers in Python

This means:
- The core agent has **zero Python dependency**
- Python users can extend via MCP or write custom tools
- ML/RL researchers use Python offline, not in the agent loop
- Enterprise deployment is just: copy one binary

---

## 6. Comparison with Original Design

| Aspect | Original (Rust+Python) | Revised (Pure Rust) |
|--------|----------------------|-------------------|
| Runtime deps | Python 3.11 + pip packages | None |
| Binary distribution | Rust binary + Python package | Single binary |
| IPC complexity | JSON-RPC over Unix socket | None needed |
| Failure modes | Python process crash, IPC timeout | Single process |
| Provider adapters | Python SDK wrappers | Raw HTTP (reqwest) |
| Planning engine | Python petgraph equivalent | Rust petgraph |
| RL collection | Python SQLite | Rust rusqlite |
| Build system | Cargo + pip + npm | Cargo only |
| Developer setup | Install Rust + Python + Node | Install Rust |

**The pure Rust design is simpler, faster, and more reliable.**

---

*Decision: cisco-code is pure Rust. Python exists only for offline analysis
and as an optional user-facing tool, never as a runtime dependency.*
