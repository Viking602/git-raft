# Architecture

- Write architecture facts here, not intent or wishful thinking.
- Keep it consistent with `src/` and `Cargo.toml`.
- Record stable decisions, module boundaries, command flow, and data flow only.
- When referencing implementation, use direct file paths.
- The current main entry point is `src/lib.rs`, and command definitions live in `src/cli.rs`.
- Event output lives in `src/events.rs`, git execution in `src/git.rs`, run persistence in `src/store.rs`, risk gating in `src/risk.rs`, and AI calls in `src/ai.rs`.
