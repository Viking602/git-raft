# Runtime

- Command entry: `src/main.rs` -> `src/lib.rs::run()`
- Main flow: parse command -> classify risk -> run git or AI -> emit events -> update run metadata
- Config resolution order: CLI overrides -> repo config -> user config -> defaults
- Commit planning uses git snapshot inspection, generated scope catalogs, configured commit format, and repo-local commit examples.
- Hook execution order is fixed: built-in rules plus matching external hooks for `beforeCommand`, `afterCommand`, `commandFailed`, `afterCommitPlan`, `beforeGroupCommit`, and `afterGroupCommit`.
- Event output supports two modes:
  - Human-readable output to the terminal by default
  - Machine-readable NDJSON with `--json`
- Fixed runtime files:
  - `.config/git-raft/config.toml`
  - `.config/git-raft/commit_examples.md`
  - `~/.config/git-raft/config.toml`
  - `.git/git-raft/runs/<run-id>/run.json`
  - `.git/git-raft/runs/<run-id>/events.ndjson`
  - optional `ai-request.json`, `ai-response.json`, and `patch.json`
