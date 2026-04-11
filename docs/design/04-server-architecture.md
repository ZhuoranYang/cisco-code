# 04 — Server Architecture: Personal Agent as a Service

## Vision

Each user gets a persistent cisco-code agent running on a server — a personal
AI assistant that knows their codebase, preferences, and work context. Users
interact via Slack, Webex, web UI, or API. The agent maintains long-running
sessions, learns from feedback, and handles jobs asynchronously.

This replaces fai-service's Python/FastAPI backend entirely. The fixed Python
workflows (PowerShell classification, MITRE mapping, alert analysis) become
cisco-code skills — prompt templates + connector calls that don't need an
agent loop.

## Architecture

```
                  ┌─────────────┐  ┌─────────────┐  ┌──────────┐
                  │  Slack Bot  │  │ Webex Bot   │  │  Web UI  │
                  └──────┬──────┘  └──────┬──────┘  └────┬─────┘
                         │                │               │
                    ┌────┴────────────────┴───────────────┴────┐
                    │              cisco-code server            │
                    │           (axum, single Rust binary)      │
                    │                                           │
                    │  ┌─────────┐  ┌──────────┐  ┌─────────┐ │
                    │  │ REST API│  │WebSocket  │  │  MCP    │ │
                    │  │  /api/* │  │ streaming │  │ server  │ │
                    │  └────┬────┘  └─────┬─────┘  └────┬────┘ │
                    │       │             │              │      │
                    │  ┌────┴─────────────┴──────────────┴───┐ │
                    │  │           Job Manager                │ │
                    │  │  (spawn, track, cancel, stream)      │ │
                    │  └────┬──────────────────────────┬─────┘ │
                    │       │                          │       │
                    │  ┌────┴────┐              ┌──────┴─────┐ │
                    │  │ Agent   │              │  Skills    │ │
                    │  │ Runtime │              │  (fixed)   │ │
                    │  │ (full   │              │            │ │
                    │  │  tool   │              │ - classify │ │
                    │  │  loop)  │              │ - mitre    │ │
                    │  │         │              │ - alert    │ │
                    │  │         │              │ - splunk   │ │
                    │  └─────────┘              └────────────┘ │
                    │                                           │
                    │  ┌──────────────────────────────────────┐│
                    │  │           Connectors / Tools          ││
                    │  │  Splunk │ VT │ GitHub │ Bash │ Files  ││
                    │  └──────────────────────────────────────┘│
                    └───────────────────────────────────────────┘
                         │              │              │
                    ┌────┴───┐    ┌────┴───┐    ┌────┴───┐
                    │PostgreSQL│   │ Valkey │    │  LLM   │
                    │(sessions│   │(cache, │    │backends│
                    │ users)  │   │ locks) │    │        │
                    └─────────┘   └────────┘    └────────┘
```

## API Design

### Jobs (agent sessions)

```
POST   /api/v1/jobs                 Submit a new agent job
GET    /api/v1/jobs                 List user's jobs
GET    /api/v1/jobs/:id             Get job status
GET    /api/v1/jobs/:id/stream      SSE stream of job events
POST   /api/v1/jobs/:id/message     Send follow-up message to running job
DELETE /api/v1/jobs/:id             Cancel a job
```

A "job" is a `ConversationRuntime` running in a tokio task. Jobs are
persistent — they survive server restarts by replaying from the JSONL
session file.

### Skills (fixed workflows, no agent loop)

```
GET    /api/v1/skills               List available skills
GET    /api/v1/skills/:name/schema  Input/output schema
POST   /api/v1/skills/:name         Execute skill synchronously
```

Skills are stateless functions: input → LLM call → output. No tool loop.
Examples: classify a PowerShell script, map a rule to MITRE, extract IOCs.

### Connections (data sources)

```
CRUD   /api/v1/connections          Manage Splunk, VT, GitHub connections
POST   /api/v1/connections/:id/test Test a connection
```

### Auth

```
POST   /api/v1/auth/token           Exchange credentials for JWT
POST   /api/v1/auth/apikey          Create API key
```

Multi-user: each user gets their own agent sessions, connections, and
API keys. Slack/Webex bots authenticate via webhook secrets.

### Webhooks (bot integration)

```
POST   /api/v1/webhooks/slack       Slack Events API endpoint
POST   /api/v1/webhooks/webex       Webex Webhooks endpoint
```

Bot receives message → creates/resumes job → streams responses back
to the channel/DM.

## Personal Agent Model

Each user's agent has:

- **Persistent memory** — preferences, project context, feedback
  (the auto-memory system already exists in the CLI)
- **Session history** — JSONL transcripts, resumable across restarts
- **Connections** — their Splunk instances, API keys, etc.
- **Skills** — shared globally, but customizable per-user
- **Model preferences** — small/medium/large class, specific models

The server manages many agents concurrently. Each agent is a tokio task
with its own `ConversationRuntime`. The job manager handles lifecycle:
spawn, park (idle timeout), resume, cancel.

## Implementation Plan

### Phase 1: HTTP skeleton
- axum server in `crates/cli/src/server.rs` (or separate `crates/server/`)
- Job submit (POST) + status (GET) + SSE streaming
- Single-user, no auth

### Phase 2: Job manager
- Spawn `ConversationRuntime` per job in tokio tasks
- Track active/completed/failed jobs
- Stream events via SSE (reuse existing `StreamEvent`)
- Session persistence (already done — JSONL)

### Phase 3: Skills
- Skill trait: `async fn run(input) -> Result<Output>`
- Port fai-service workflows as skills
- Splunk connector in Rust

### Phase 4: Multi-user + auth
- JWT auth with user isolation
- Per-user connections, sessions, memory
- API key management

### Phase 5: Bot integration
- Slack Events API handler
- Webex Webhooks handler
- Message → job mapping (new job or resume existing)
- Response chunking for chat message limits

### Phase 6: MCP server
- Expose tools and skills via MCP protocol
- Other agents can use cisco-code as a tool provider

## Technology Choices

- **HTTP framework**: axum (already in Cargo.toml dependencies)
- **Streaming**: SSE via axum's `Sse` type, or WebSocket via tokio-tungstenite
- **Database**: PostgreSQL via sqlx (for users, connections, job metadata)
  - Session content stays in JSONL files (append-only, fast)
- **Cache**: Valkey/Redis via fred or deadpool-redis (rate limits, locks)
- **Auth**: JWT via jsonwebtoken crate
- **Bot SDKs**: Raw HTTP (Slack/Webex APIs are simple REST)

## Migration from fai-service

| fai-service component | cisco-code equivalent |
|---|---|
| FastAPI app | axum server |
| LangGraph workflows | Skills (stateless functions) |
| Splunk connector (Python) | Rust SplunkConnector tool |
| MCP server (FastMCP) | Built-in MCP server |
| PostgreSQL models | sqlx models |
| LLM providers | Already done (Bedrock, OpenAI, Anthropic, local) |
| React frontend | Keep as-is, point at cisco-code server |

The React frontend from fai-service can be reused with minimal changes —
just update API endpoints to point at cisco-code server instead of FastAPI.
