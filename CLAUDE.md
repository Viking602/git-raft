# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`git-raft` is a Rust CLI that provides AI-powered Git operations (commit planning, merge/rebase conflict resolution). See @AGENTS.md for the execution index and source-of-truth rules.

## Build & Test

```bash
make build          # cargo build
make test           # cargo test (all tests)
make cli-test       # cargo test --test cli (CLI behavior tests)
make guardrails     # cargo test --test guardrails (structural tests)
make fmt            # cargo fmt
make fmt-check      # cargo fmt --check
```

Run `cargo clippy -- -D warnings` before considering work complete.

## Architecture

- Entry: `src/main.rs` → `src/lib.rs::run()` → `src/app/dispatch.rs`
- Commands: `src/commands/` (commit, branch, merge_rebase)
- AI integration: `src/ai/` (OpenAI-compatible provider)
- Config: `src/config/` (file-based, merged from CLI → repo → user → defaults)
- Hooks: `src/hooks/` (built-in rules + external executables)
- Docs: `docs/` (architecture, product, quality, exec-plans)
- Config file: `.config/git-raft/config.toml` (repo-level)

## Environment Variables

- `GIT_RAFT_BASE_URL` — AI provider base URL
- `GIT_RAFT_API_KEY` — AI provider API key
- `GIT_RAFT_MODEL` — AI model selection

## Conventions

- Rust edition 2024
- Conventional commits (e.g., `feat(commit):`, `fix(ai):`, `refactor:`, `docs:`)
- Work directly on master
- Keep AGENTS.md under 120 lines (enforced by guardrails test)
- New commands or paths require doc index and structural test updates
