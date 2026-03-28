# Runtime

- Command entry: `src/main.rs` -> `src/lib.rs::run()`
- Main flow: parse command -> `src/app/dispatch.rs` -> classify risk -> run command workflow -> emit events -> update run metadata
- The current CLI surface is limited to `commit`, `merge`, and `rebase`
- `commit` workflow lives in `src/commands/commit/`
- `merge` and `rebase` workflow lives in `src/commands/merge_rebase.rs`
- Config resolution order is:
  - CLI overrides for supported agent commands
  - repo config
  - user config
  - defaults
- Config files are read when present.
  - `.config/git-raft/config.toml`
  - `~/.config/git-raft/config.toml`
- Config schema, path resolution, file loading, and merge rules live behind `src/config.rs` with helpers in `src/config/`.

- AI request flow is task-scoped.
  - conflict resolution builds a structured AI request and expects strict patch JSON
  - conflict resolution only accepts text conflicts; non-text conflicted files stop for manual review before any AI apply
  - conflict resolution can retry once after a rejected candidate by sending the previous candidate plus a failure report back to the model
  - `commit` builds a structured AI planning request and expects strict `CommitPlan` output through the `plan_commit` tool call
  - `commit` planning includes an AI grouping decision and grouping confidence gate
  - if the AI does not clear the split threshold, runtime execution collapses the plan to one commit automatically
  - the model response is recorded before any patch application happens
  - merge/rebase candidate patches go through host-side retention checks first, then configured verification commands in a temporary worktree copy
  - file writes and `git add` stay in the host runtime and only happen after those checks pass

- Hook execution order is fixed: built-in rules plus matching external hooks for `beforeCommand`, `afterCommand`, `commandFailed`, `afterCommitPlan`, `beforeGroupCommit`, `afterGroupCommit`, `beforeAiRequest`, `afterAiResponse`, and `beforePatchApply`
- `src/ai.rs`, `src/config.rs`, `src/git.rs`, and `src/hooks.rs` are now facade modules with their heavier internals split into matching subdirectories.

- Event output supports two modes:
  - human-readable output to the terminal by default
  - machine-readable NDJSON with `--json`

- AI lifecycle events include `ai_request_started`, `ai_response_ready`, `ai_response_invalid`, `ai_patch_ready`, and `ai_patch_applied`

- Fixed runtime files:
  - `.git/git-raft/runs/<run-id>/run.json`
  - `.git/git-raft/runs/<run-id>/events.ndjson`
  - optional `ai-request.json`, `ai-response.json`, `patch.json`, and `validation.json`

- `ai-request.json` stores the task name, the structured AI request, and the raw provider request body
- `ai-response.json` stores the task name, the normalized AI response, a response summary, and the raw provider response body
- `patch.json` stores the last AI merge candidate
- `validation.json` stores each AI merge attempt, its rejection reason, and the verification command results
