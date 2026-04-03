# git-raft

AI-powered Git operations for commit planning, merge/rebase conflict resolution, and author management.

`git-raft` sits between you and Git. It reads your staged and unstaged changes, sends them to an AI provider, and comes back with structured commit plans or conflict resolutions — then executes them.

## Features

- **Smart commit planning** — AI groups your changes into logical commits with conventional commit messages. Splits only when it's confident each commit stands on its own.
- **AI conflict resolution** — `merge` and `rebase` detect conflicts and ask AI to resolve them. Candidates are validated against retention checks and configurable verification commands before being applied.
- **Author rewriting** — Set a project-level author and rewrite recent commits that used the wrong identity.
- **Hook system** — Built-in rules plus external hook executables driven by structured JSON payloads. Hook into AI request/response lifecycle events.
- **Configurable** — File-based config at repo and user level. Supports multiple commit formats (conventional, angular, gitmoji, simple), languages (en, zh), and custom ignore paths.

## Requirements

- Rust 2024 edition (1.85+)
- An OpenAI-compatible API endpoint

## Installation

```bash
cargo install --path .
```

## Quick Start

Set up your AI provider:

```bash
export GIT_RAFT_BASE_URL="https://api.openai.com/v1"
export GIT_RAFT_API_KEY="sk-..."
export GIT_RAFT_MODEL="gpt-4o"
```

Or use the repo-level config file at `.config/git-raft/config.toml`:

```toml
[provider]
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
model = "gpt-4o"
```

## Usage

### Commit

```bash
# Plan and execute commits
git-raft commit

# Preview the plan without committing
git-raft commit --plan

# Dry run — show what would happen
git-raft commit --dry-run

# Guide the AI with intent
git-raft commit --intent "focus on the auth refactor"

# Override commit language
git-raft commit --lang zh
```

### Merge

```bash
# Merge with AI conflict resolution (--apply-ai is default)
git-raft merge feature-branch

# Pass extra args to git merge
git-raft merge feature-branch --no-ff
```

### Rebase

```bash
# Rebase with AI conflict resolution
git-raft rebase main
```

### Branch

```bash
# Create and switch to a new branch from a commit
git-raft branch my-feature abc1234
```

### Purge

```bash
# Remove a file from working tree and entire git history
git-raft purge secret.env

# Remove multiple paths
git-raft purge .env credentials.json build/

# Allow rewriting already-pushed commits
git-raft purge secret.env --force

# Rewrite and force push (requires --yes confirmation)
git-raft purge secret.env --force --push --yes
```

### Author

```bash
# Set project author and rewrite mismatched recent commits
git-raft author --name "Your Name" --email "you@example.com"

# Force rewrite already-pushed commits
git-raft author --name "Your Name" --email "you@example.com" --force

# Force rewrite and push
git-raft author --name "Your Name" --email "you@example.com" --force --push
```

## Configuration

### Commit Settings

```toml
[commit]
format = "conventional"    # conventional | angular | gitmoji | simple
language = "en"            # en | zh
use_gitmoji = false
include_body = true
include_footer = false
ignore_paths = ["docs/generated", ".local"]
```

### Merge / Rebase Verification

```toml
[merge]
repair_attempts = 1

[[merge.verification]]
program = "cargo"
args = ["build"]

[[merge.verification]]
program = "cargo"
args = ["test"]
```

### Hooks

```toml
[hooks.rules]
# Built-in rules configuration

[[hooks.external]]
# External hook executables
```

Hook events: `afterCommitPlan`, `beforeGroupCommit`, `afterGroupCommit`, `beforeAiRequest`, `afterAiResponse`, `beforePatchApply`.

## Global Flags

| Flag | Description |
|------|-------------|
| `--json` | Emit newline-delimited JSON events |
| `--yes` | Skip confirmation prompts for high-risk operations |

## Environment Variables

| Variable | Description |
|----------|-------------|
| `GIT_RAFT_BASE_URL` | AI provider base URL |
| `GIT_RAFT_API_KEY` | AI provider API key |
| `GIT_RAFT_MODEL` | AI model selection |

## Development

```bash
cargo build              # Build
cargo test               # All tests
cargo test --test cli    # CLI behavior tests
cargo test --test guardrails  # Structural guardrails
cargo clippy -- -D warnings   # Lint
cargo fmt                # Format
```

## License

[MIT](LICENSE)
