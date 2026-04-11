---
name: claude-api
description: Build apps with the Claude API or Anthropic SDK
user-invocable: true
---

Help build applications using the Claude API or Anthropic SDK. Trigger when code imports `anthropic`, `@anthropic-ai/sdk`, or `claude_agent_sdk`, or when the user asks about Claude API usage.

Key patterns:
- Messages API with streaming
- Tool use / function calling
- Vision (image inputs)
- Extended thinking
- Prompt caching
- Batch API
- Agent SDK patterns

Use the latest model IDs: claude-opus-4-6, claude-sonnet-4-6, claude-haiku-4-5.
Default to claude-sonnet-4-6 for most applications.
