---
name: plan
description: Enter plan mode to explore the codebase and design an implementation approach before coding
user-invocable: true
---

# /plan — design before you build

Enter plan mode for complex tasks that require exploration and design before implementation.

## When to use

Use plan mode when:
- The task touches multiple files or modules
- You need to understand existing patterns before making changes
- There are multiple possible approaches and trade-offs to consider
- The user asks you to "think about", "plan", or "design" something first

Do NOT use plan mode for:
- Simple, well-scoped changes (single file edits, typo fixes)
- Tasks where the approach is obvious
- When the user says "just do it" or wants immediate action

## Entering plan mode

Call the EnterPlanMode tool. This switches to a read-only exploration phase.

In plan mode you should:
1. **Explore the codebase** thoroughly — read relevant files, understand existing patterns
2. **Identify similar features** and architectural approaches already in use
3. **Consider multiple approaches** and their trade-offs
4. **Use AskUserQuestion** if you need to clarify requirements or approach
5. **Design a concrete implementation strategy** with specific files to create/modify

**DO NOT write or edit any files in plan mode.** This is a read-only exploration and planning phase.

## The plan file

Write your plan to a markdown file at `.cisco-code/plans/<slug>.md` (or the configured plans directory). The plan should include:

```markdown
# Plan: <title>

## Goal
<1-2 sentence summary of what we're building>

## Approach
<The chosen approach and why>

## Changes
- [ ] `path/to/file.rs` — description of change
- [ ] `path/to/other.rs` — description of change
- [ ] `path/to/new_file.rs` — new file, purpose

## Considerations
- <trade-off or risk>
- <alternative considered and why rejected>
```

## Exiting plan mode

When the plan is ready, call ExitPlanMode to present it for user approval. The user can:
- **Approve**: proceed to implementation using the plan as a guide
- **Request changes**: refine the plan before proceeding
- **Reject**: abandon the plan

After approval, implement the plan step by step, checking off items as you complete them.

## Arguments

- `/plan` — enter plan mode (no args)
- `/plan open` — view the current session's plan file
- `/plan <description>` — enter plan mode with a specific goal
