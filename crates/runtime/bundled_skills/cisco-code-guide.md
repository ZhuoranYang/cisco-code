---
name: cisco-code-guide
description: Answer questions about cisco-code features, configuration, tools, MCP servers, hooks, and commands
user-invocable: true
---

When the user asks about cisco-code capabilities, features, or configuration, search the codebase for relevant implementation details and provide concrete answers with examples.

## How to answer

1. Search the codebase for the relevant topic:
   - Tool definitions: look in `crates/runtime/src/tools/` and `ToolRegistry`
   - Slash commands and skills: look in `crates/runtime/bundled_skills/`
   - Hook system: look in `crates/runtime/src/hooks/` for events (PreToolUse, PostToolUse, Notification, etc.) and configuration
   - MCP servers: look in `crates/runtime/src/mcp/` for dynamic tool registration and server management
   - Notifications: look in `crates/runtime/src/notifications/` for channel implementations
   - Daemon/server/attach modes: look in `crates/cli/src/` for entry points and mode flags
   - Cron scheduling: look for cron-related modules in `crates/runtime/src/cron/` or `crates/cli/`
   - Session management: look in `crates/runtime/src/session/`
   - Config: look for `settings.json` parsing in `crates/runtime/src/config/`
2. Read the actual source files to give accurate, up-to-date answers.
3. Provide config snippets and examples where helpful.
4. Reference file paths so the user can explore further.

## Topics to cover

- **Tools**: Built-in tools registered in ToolRegistry (Bash, Read, Write, Edit, Grep, Glob, etc.) and how MCP servers add dynamic tools.
- **Slash commands and skills**: Bundled skills in `bundled_skills/`, how to create custom skills, the YAML frontmatter format.
- **Hook system**: Events (PreToolUse, PostToolUse, Notification, SessionStart, SessionEnd), how to configure hooks in `settings.json`, example hook scripts.
- **MCP servers**: How to add servers in `settings.json` under `mcpServers`, stdio vs SSE transports, dynamic tool discovery.
- **Notification channels**: Webhook, Slack, Webex, Console, TerminalBell, OsNotification -- how to configure each.
- **Daemon mode**: Running cisco-code as a background daemon, attaching to sessions.
- **Server mode**: HTTP/API server mode for programmatic access.
- **Attach mode**: Connecting to an existing running session.
- **Cron scheduling**: Setting up recurring tasks with CronCreate/CronDelete/CronList.
- **Session management**: Persistence, resume, session listing.
- **Configuration**: `settings.json` (project `.cisco-code/settings.json` and user `~/.cisco-code/settings.json`), `CLAUDE.md` / memory files, permission modes.

Always ground answers in what the code actually implements rather than speculating.
