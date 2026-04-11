---
name: commit
description: Create a git commit with AI-generated message
user-invocable: true
---

Look at the current git diff (both staged and unstaged changes). Create a well-crafted git commit following conventional commit format. Stage relevant files and commit with an appropriate message.

Follow these steps:
1. Run `git status` and `git diff` to see all changes
2. Run `git log --oneline -5` to see recent commit message style
3. Stage relevant files (prefer specific files over `git add -A`)
4. Write a concise commit message that focuses on the "why" not the "what"
5. Create the commit
