# Architecture

- Write architecture facts here, not intent or wishful thinking.
- Keep it consistent with `src/` and `Cargo.toml`.
- Record stable decisions, module boundaries, command flow, and data flow only.
- When referencing implementation, use direct file paths.
- The current main entry point is `src/lib.rs`, and command definitions live in `src/cli.rs`.
- Config loading and merge lives behind `src/config.rs`, with internal helpers under `src/config/`.
- Merge/rebase safety policy is configured under the top-level `[merge]` block in repo or user config.
- Runtime dispatch lives in `src/app/dispatch.rs`.
- Command workflows live under `src/commands/`.
- Event output lives in `src/events.rs`, git execution lives behind `src/git.rs`, run persistence in `src/store.rs`, risk gating in `src/risk.rs`, and AI calls live behind `src/ai.rs`.
