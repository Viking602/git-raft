# Real Merge

- Purpose: create a real conflict fixture repo and run `git-raft merge` against it with a real model.
- Scope: `merge` only.
- The helper scripts live under `scripts/real-merge/`.

## Prerequisites

- `git`
- `cargo`
- `chub`
- a local clone of `Viking602/tests`
- a built `git-raft` binary
- `GIT_RAFT_API_KEY` in the shell environment

`chub` is a repo rule for code changes. These helper scripts do not call `chub`; run it yourself before changing Rust code.

## Fixture Setup

Create or clone a clean local copy of `Viking602/tests`, then run:

```bash
scripts/real-merge/setup_viking602_tests.sh /path/to/tests
```

This creates these branches:

- `master`
- `feature/format-report`
- `scenario/validation-target`
- `scenario/validation-feature`
- `scenario/repair-target`
- `scenario/repair-feature`
- `scenario/binary-target`
- `scenario/binary-feature`

## Run The Suite

Build `git-raft`, then run:

```bash
export GIT_RAFT_API_KEY='...'
export GIT_RAFT_BASE_URL='https://openrouter.ai/api/v1'
export GIT_RAFT_MODEL='xiaomi/mimo-v2-flash'

scripts/real-merge/run_scenarios.sh /path/to/tests /path/to/git-raft
```

The runner creates a fresh clone per scenario, writes `.config/git-raft/config.toml`, executes the merge, and copies the latest `.git/git-raft/runs/<run-id>/` into the result directory.

## Result Paths

- Suite summary:
  - `<sample-repo>/.git/git-raft-real-merge/<timestamp>/summary.md`
- Per-scenario copies:
  - `<sample-repo>/.git/git-raft-real-merge/<timestamp>/<scenario>/`
- Copied runtime artifacts:
  - `run/run.json`
  - `run/events.ndjson`
  - `run/ai-request.json`
  - `run/ai-response.json`
  - `run/patch.json`
  - `run/validation.json`

## Scenarios

- `A`: plain `git merge` baseline. It must stop on conflicts in `src/lib.rs` and `tests/math_merge.rs`.
- `B`: `master` vs `feature/format-report`. Two branches now preserve separate public helpers and a stable summary entrypoint. Expected path is auto-apply plus green validation.
- `C`: validation-stop scenario. Fixture itself is mergeable; the stop is forced by an extra verification script that rejects public API drift after retention passes.
- `D`: repair-retry scenario. Expected path is at least two attempts in `validation.json`.
- `E`: binary conflict scenario. Expected path is `conflict files must be decodable text`.
