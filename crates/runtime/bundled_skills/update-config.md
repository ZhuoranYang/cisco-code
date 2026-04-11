---
name: update-config
description: Configure cisco-code settings via settings.json
user-invocable: true
---

Help the user configure cisco-code settings. Settings are stored in:
- Project: `.cisco-code/settings.json`
- User: `~/.cisco-code/settings.json`

Available settings:
- `permissions.defaultMode`: default, accept-reads, bypass, deny-all
- `permissions.allow/deny`: tool permission rules
- `hooks.PreToolUse/PostToolUse`: hook commands
- `mcpServers`: MCP server configurations
- `model`: default model override
- `autoCompactEnabled`: enable/disable auto-compaction
- `autoMemoryEnabled`: enable/disable auto-memory

Read the current settings, help the user understand what can be configured, and write the updated settings file.
