---
name: remember
description: Save information to persistent memory for future conversations
user-invocable: true
---

Save information the user wants to remember across conversations. Write it to the memory system at `.cisco-code/memory/`.

Each memory file uses this format:
```markdown
---
name: {{memory name}}
description: {{one-line description}}
type: {{user, feedback, project, reference}}
---

{{memory content}}
```

Steps:
1. Determine the type of memory (user, feedback, project, reference)
2. Write the memory file to `.cisco-code/memory/`
3. Update `MEMORY.md` index with a pointer to the new file
