---
name: init-project
description: Initialize a new project with best-practice structure and tooling
user-invocable: true
---

# /init-project — scaffold a new project

Set up a new project with idiomatic directory structure, tooling, and git.

## Steps

1. **Ask the user** what kind of project they want (if not already specified):
   - Language/framework: Rust, Python, Node.js, Go, or other
   - Project name (default: current directory name)

2. **Create the project** using the standard toolchain for the language:

   **Rust**
   - `cargo init` (in current dir) or `cargo new <name>`
   - Produces Cargo.toml, src/main.rs (or lib.rs), .gitignore

   **Python**
   - Create `pyproject.toml` with project metadata and basic config for ruff, pytest
   - Create `src/<pkg>/`, `src/<pkg>/__init__.py`, `tests/`, `tests/__init__.py`
   - `python3 -m venv .venv`

   **Node.js / TypeScript**
   - `npm init -y`
   - Create `src/`, `tests/`
   - If TypeScript: add `tsconfig.json` with strict defaults, add `typescript` and `@types/node` to devDependencies

   **Go**
   - `go mod init <module-path>`
   - Create `cmd/<name>/main.go`, `internal/`, `pkg/`

   For other languages, create a sensible minimal scaffold and ask the user to confirm.

3. **Common tooling** (add only if not already present):
   - `.gitignore` — language-appropriate ignores
   - `.editorconfig` — utf-8, LF, 4-space indent (2-space for JS/TS/YAML)
   - `.github/workflows/ci.yml` — minimal CI: install deps, lint, test

4. **Create `CLAUDE.md`** at the project root with:
   - Project name and one-line description
   - Build / test / lint commands
   - Key directory layout
   - Any conventions chosen above

5. **Initialize git** if not already in a repository:
   - `git init`
   - Create an initial commit with the scaffolded files

## Guidelines

- Prefer a single toolchain command (cargo init, npm init) over manual file creation when available.
- Keep generated files minimal — no boilerplate the user will immediately delete.
- Do NOT install heavy frameworks or dependencies unless the user asks.
- If any step fails (e.g., toolchain not installed), warn the user and continue with manual setup.
