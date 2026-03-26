# AGENTS.md

## Role
- This is the working root for `git-raft`.
- This repository is a Rust CLI project for `git-raft`.
- Keep this file as an execution index, not a handbook.
- Code, `Cargo.toml`, tests, commits, and run artifacts are more trustworthy than prose.

## Source Of Truth
- When docs and code disagree, trust the actual repository state.
- Do not guess paths or commands.
- Read the relevant source first, then read docs.
- When checking current state, start with `git status` and recent changes.
- If you see unfinished user work, preserve it. Do not revert it.

## Read Here First
- `Cargo.toml`: package name, version, edition, dependencies.
- `src/`: current implementation.
- `docs/architecture/`: architecture facts.
- `docs/product/`: product behavior and scope.
- `docs/quality/`: verification entry points.
- `docs/exec-plans/active/`: plans in progress.
- `docs/exec-plans/completed/`: completed plans.
- `docs/generated/`: trace, run data, exports, and other generated output.

## Trace And Run Data
- Check existing generated artifacts in the repository before reading docs.
- Runtime trace, events, and run metadata are stored at `.git/git-raft/runs/<run-id>/`.
- The repository-local config file is stored at `.config/git-raft/config.toml`.
- The optional user-level config file is stored at `~/.config/git-raft/config.toml`.
- Commit message examples live at `.config/git-raft/commit_examples.md`.
- For one run, read `run.json` and `events.ndjson` first.
- `docs/generated/` documents those paths and their purpose. It is not runtime storage.
- Keep only the minimum information needed to reproduce a problem.

## Verification Entry Points
- Default entry points are `cargo build`, `cargo test`, and `cargo run`.
- For CLI behavior, start with `cargo test --test cli`.
- For structural guardrails, run `cargo test --test guardrails`.
- If more specialized checks are added later, document them in `docs/quality/`.
- Start with the shortest path, then add heavier checks if needed.
- Only report results you actually ran.

## Dangerous Operations
- Unless explicitly requested, do not do these:
  - `git reset --hard`
  - `git checkout --`
  - force push
  - history deletion
  - bulk file deletion
- If you need to touch someone else's ongoing work, confirm the scope first.
- Only change files required for the current task. Do not refactor extra code.

## How To Add Rules
- Add new rules to the closest topic-specific doc first.
- If a rule comes from a concrete fact, include the source file path.
- When a rule changes, update the body before the index.
- If long-term value is unclear, put it under `docs/generated/` or `docs/exec-plans/active/`.
- Move stale content to `docs/exec-plans/completed/` or delete it.

## Plan Files
- Plan files should record the task, not process chatter.
- Each plan should include goal, scope, and verification method.
- When a plan changes, update the timestamp instead of creating duplicates.
- Completed plans should make the final outcome visible.
- If the task changes midstream, update the plan first.

## Document Maintenance
- Before adding a doc, decide which directory owns it.
- Update the directory index before the body when possible.
- Do not duplicate the same fact in multiple places.
- Use stable file names for material that will be referenced long term.
- Keep temporary notes out of architecture and product pages.

## Failure Handling
- If facts are unclear, stop and inspect the repository instead of guessing.
- If someone else is editing the same area, preserve the current state and explain the overlap.
- If docs and implementation diverge, fix the doc entry points and indexes first, then decide whether the body needs changes.

## Writing Style
- Keep sentences short.
- Refer to concrete file paths.
- Avoid filler and long explanations.
- If an example is needed, use the smallest one.
- If a conclusion is needed, state it directly.

## Current Boundary
- The repository already has a runnable Rust CLI, event stream, run persistence, risk gate, config show/get/set, scopes generate/list, commit planning, doctor, runs, and rollback.
- The merge/rebase AI conflict path is wired to `GIT_RAFT_BASE_URL`, `GIT_RAFT_API_KEY`, and `GIT_RAFT_MODEL`.
- Hooks support built-in rule checks plus external executables driven by a structured JSON payload.
- Before adding a new command or path, update the doc index and structural tests.
- Keep `AGENTS.md` short. Do not turn the root file back into a long manual.
