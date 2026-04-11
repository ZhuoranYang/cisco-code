---
name: verify
description: Verify that recent changes work correctly by running tests and checks
user-invocable: true
---

Verify that recent code changes are correct:

1. Identify what was recently changed (git diff or session context)
2. Run the relevant test suite (cargo test, npm test, pytest, etc.)
3. Run any linters or type checkers configured in the project
4. Check for compilation errors
5. Report results clearly: what passed, what failed, and any issues found
