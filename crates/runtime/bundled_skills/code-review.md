---
name: code-review
description: Code review a pull request or set of changes
user-invocable: true
---

Review the code changes for bugs, security issues, performance problems, and improvements.

Steps:
1. Identify the changes (git diff, PR diff, or specified files)
2. Review each changed file for:
   - Logic errors and edge cases
   - Security vulnerabilities (OWASP top 10)
   - Performance issues
   - Code style and readability
   - Missing error handling
   - Test coverage gaps
3. Provide specific, actionable feedback with file:line references
4. Categorize findings as: critical, warning, suggestion, or nitpick
