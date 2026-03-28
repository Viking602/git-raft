# CLI

- Binary name: `git-raft`
- Global flags:
  - `--json`: emit NDJSON event output
  - `--yes`: confirm high-risk operations
- Current commands:
  - `commit`
  - `merge`
  - `rebase`

- Configuration is file-based.
  - Repository-local config file: `.config/git-raft/config.toml`
  - Optional user-level config file: `~/.config/git-raft/config.toml`
  - Repository-local commit example file: `.config/git-raft/commit_examples.md`

- Provider selection and authentication:
  - Model selection is configured under `[provider].model`
  - Base URL can come from `[provider].base_url` or `GIT_RAFT_BASE_URL`
  - Authentication can use either `[provider].api_key` or `[provider].api_key_env`
  - If `api_key` is empty, `git-raft` reads the environment variable named by `api_key_env`
  - `GIT_RAFT_MODEL` overrides `[provider].model`

- `commit` is planner-driven.
  - `commit --plan` requests the AI planner and prints a non-mutating plan
  - `commit --dry-run` requests the AI planner and previews the result without creating commits
  - `commit --intent <text>` is passed to the AI planner as extra guidance
  - `commit --language <en|zh>` overrides the configured commit subject language for one run
  - staged and unstaged changes are planned together by default
  - `commit` requires a configured AI provider because grouping and commit messages are AI-generated
  - The AI must explicitly decide whether the change should stay as one commit or be split
  - Split execution is only accepted when the AI grouping confidence clears the built-in threshold

- Commit format behavior:
  - Commit message format selection is configured under `[commit].format`
  - Built-in presets are `conventional`, `angular`, `gitmoji`, and `simple`
  - Commit subject language is configured under `[commit].language`
  - Built-in language values are `en` and `zh`
  - `use_gitmoji` can be enabled under `[commit]`
  - Commit planner ignore rules are configured under `[commit].ignore_paths`
  - Commit body generation is configured under `[commit].include_body`
  - Commit footer generation is configured under `[commit].include_footer`

- `merge` and `rebase` can request AI conflict resolution.
  - `--apply-ai` defaults to `true`
  - The model returns a structured patch proposal with full file contents for conflicted files
  - Host runtime checks that the AI result keeps unique non-duplicate conflict content before any write happens
  - Host runtime runs configured verification commands in a temporary workspace before any write happens
  - Candidate patches are saved to `patch.json` even when they are rejected for manual review
  - `validation.json` records each attempt, rejection reason, and verification command result
  - AI confidence is telemetry only; auto-apply requires retention checks, configured verification, and a clean candidate
  - When `[merge].verification` is empty, AI can still produce a candidate, but `git-raft` will stop for manual review instead of applying it

- Merge verification is file-based config.
  - `merge` and `rebase` share the top-level `[merge]` config block
  - `repair_attempts` controls how many AI repair retries are allowed after the first candidate fails checks
  - `[[merge.verification]]` defines structured verification commands as `program + args`
  - Verification commands run from a temporary copy of the repo root without `.git`
  - Validation commands must not depend on Git metadata
  - Example:

```toml
[merge]
repair_attempts = 1

[[merge.verification]]
program = "cargo"
args = ["build"]

[[merge.verification]]
program = "cargo"
args = ["test", "--test", "cli"]
```

- Hook support:
  - built-in rules live under `[hooks.rules]`
  - external hooks live under `[[hooks.external]]`
  - hook payloads use camelCase JSON keys
  - AI-specific external hook events are `beforeAiRequest`, `afterAiResponse`, and `beforePatchApply`
  - AI hook payloads can include `agentTask`, `agentRequestSummary`, `agentResponseSummary`, and `patchConfidence`
  - `beforePatchApply` only fires after the AI candidate passes retention checks and verification commands

- Removed from the CLI surface:
  - `ask`
  - passthrough git subcommands such as `status`, `diff`, `add`, `branch`, `switch`, `stash`, and `log`
  - support commands such as `sync`, `init`, `rollback`, `runs`, `trace`, `doctor`, `config`, and `scopes`
