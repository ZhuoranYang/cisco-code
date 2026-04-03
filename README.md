# cisco-code

An enterprise AI coding agent by Cisco, built in **pure Rust**.

Ships as a single binary with zero runtime dependencies. No Node.js, no Python, no pip install.

Synthesizes the best architectural ideas from:
- **Claude Code** — rich tool orchestration, permission system, mature agent loop
- **Codex** — OS-native sandboxing (Seatbelt/Bubblewrap), Rust performance, transport fallback
- **Astro-Assistant** — DAG planning, 5-tier model routing, RL trajectory collection, 10+ providers
- **Claw-Code-Parity** — clean generic `ConversationRuntime<P, T>` in Rust

## Architecture

```
                         cisco-code (single Rust binary)
┌─────────────────────────────────────────────────────────────────┐
│  CLI / TUI (Ratatui)                                            │
│  Agent Loop (ConversationRuntime<P>)                            │
│  Tool System (35+ built-in tools + MCP + Cisco tools)           │
│  Permission Engine + Hook System + Guardian LLM                 │
│  OS-Native Sandbox (Seatbelt / Bubblewrap / Windows tokens)     │
│  Provider Registry (Anthropic, OpenAI, Google, Bedrock, 10+)    │
│  5-Tier Model Routing (SMALL → FRONTIER)                        │
│  DAG Planning Engine (parallel wave execution + replanning)     │
│  Session Persistence (JSONL) + Memory (SQLite+FTS5)             │
│  RL Trajectory Collection + Enterprise Audit (Splunk)           │
└─────────────────────────────────────────────────────────────────┘
    LLM APIs are just HTTP — reqwest handles everything natively.
```

## Quick Start

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Build
cargo build --release

# Run
./target/release/cisco-code doctor
./target/release/cisco-code prompt "explain this codebase"
./target/release/cisco-code  # interactive REPL
```

## Key Features

| Feature | Description |
|---------|-------------|
| **Pure Rust** | Single binary, <100ms startup, zero dependencies |
| **Multi-Model** | 10+ LLM providers via raw HTTP (Anthropic, OpenAI, Google, Bedrock, Cisco AI, Ollama, vLLM) |
| **5-Tier Routing** | Right model for each task (SMALL→FRONTIER), 80%+ cost savings |
| **DAG Planning** | Complex tasks decomposed into parallel-executable task graphs |
| **OS-Native Sandbox** | Seatbelt (macOS), Bubblewrap+Landlock (Linux), restricted tokens (Windows) |
| **Guardian LLM** | Second opinion on dangerous operations |
| **35+ Tools** | bash, file ops, search, web, agent, memory, plan, notebook, LSP |
| **Cisco Tools** | Webex, DNAC, Meraki, SecureX/XDR, Duo, ISE |
| **Enterprise Auth** | SAML/OIDC SSO + Duo MFA |
| **RL-Ready** | Every step records (state, action, observation, reward) traces |
| **MCP Support** | Full Model Context Protocol client and server |
| **Compliance** | PII detection, secrets scanning, export control |

## Documentation

- [Architectural Comparison](docs/comparisons/00-architectural-comparison.md) — Deep analysis of Claude Code, Codex, Astro, Claw
- [Grand Design](docs/design/01-grand-design.md) — cisco-code architecture and design principles
- [Implementation Roadmap](docs/design/02-implementation-roadmap.md) — 10-phase step-by-step plan
- [Why Pure Rust](docs/design/03-why-pure-rust.md) — Language decision analysis

## Project Structure

```
cisco-code/
├── crates/              # Rust workspace (10 crates)
│   ├── cli/             # Binary: REPL, TUI, args
│   ├── runtime/         # Core: agent loop, session, permissions
│   ├── tools/           # Tool registry + 35+ built-in tools
│   ├── providers/       # LLM provider adapters (pure Rust HTTP)
│   ├── planning/        # DAG planner, executor, replanner
│   ├── sandbox/         # OS-native sandboxing
│   ├── api/             # HTTP client, SSE, WebSocket
│   ├── mcp/             # Model Context Protocol
│   ├── protocol/        # Shared types & events
│   └── telemetry/       # Observability & RL traces
├── config/              # Default configuration (TOML)
├── skills/              # Built-in skill definitions
├── scripts/             # Offline tools (Python, optional, not runtime)
└── docs/                # Design documentation
```

## License

MIT
