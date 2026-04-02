# Author Rewrite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `git-raft author` subcommand that sets project-level author and rewrites recent commits with incorrect author info.

**Architecture:** New `CommandKind::Author` variant dispatched through existing `dispatch.rs`. Core logic in `src/commands/author.rs`. Git operations (log scanning, rebase rewrite, force push) added to `src/git/worktree.rs`.

**Tech Stack:** Rust, clap (CLI parsing), tokio (async git process execution), tempfile (tests)

---

### Task 1: CLI — Add `CommandKind::Author` variant

**Files:**
- Modify: `src/cli.rs:24-89` (add Author variant to CommandKind enum)

- [ ] **Step 1: Write the failing test**

Add to `tests/cli.rs` after the `branch_help_lists_name_and_commit_arguments` test:

```rust
#[test]
fn author_help_lists_name_email_and_force_flags() {
    let stdout = help_output(&["author", "--help"]);
    assert!(stdout.contains("Set project-level commit author"));
    assert!(stdout.contains("--name"));
    assert!(stdout.contains("--email"));
    assert!(stdout.contains("--force"));
    assert!(stdout.contains("--push"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test cli author_help_lists_name_email_and_force_flags`
Expected: FAIL — `CommandKind` has no `Author` variant

- [ ] **Step 3: Add Author variant to CommandKind**

In `src/cli.rs`, add this variant inside the `CommandKind` enum (after the `Rebase` variant):

```rust
    /// Set project-level commit author and rewrite recent commits with wrong author.
    Author {
        /// Author name for this project.
        #[arg(long)]
        name: String,
        /// Author email for this project.
        #[arg(long)]
        email: String,
        /// Allow rewriting commits that have already been pushed to the remote.
        #[arg(long)]
        force: bool,
        /// Force push to remote after rewriting (requires --force).
        #[arg(long)]
        push: bool,
    },
```

Update the `label()` method in `impl CommandKind`:

```rust
    Self::Author { .. } => "author",
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --test cli author_help_lists_name_email_and_force_flags`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs tests/cli.rs
git commit -m "feat(author): add Author variant to CLI"
```

---

### Task 2: Risk classification for Author command

**Files:**
- Modify: `src/risk.rs:24-30` (add Author match arm)

- [ ] **Step 1: Write the failing test**

Create `tests/cli_cases/author.rs` with:

```rust
use super::*;

#[test]
fn author_with_force_push_requires_confirmation() {
    let repo = init_repo();
    let output = run_agent(
        repo.path(),
        &["--json", "author", "--name", "Test", "--email", "t@t.com", "--force", "--push"],
    );
    assert!(!output.status.success(), "author --force --push should require --yes");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("\"event_type\":\"awaiting_confirmation\""));
}

#[test]
fn author_without_force_push_does_not_require_confirmation() {
    let repo = init_repo();
    let output = run_agent(
        repo.path(),
        &["--json", "author", "--name", "Test", "--email", "t@t.com"],
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(!stdout.contains("\"event_type\":\"awaiting_confirmation\""));
}
```

Register the module in `tests/cli.rs` after the `commit_cases` module:

```rust
#[path = "cli_cases/author.rs"]
mod author_cases;
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --test cli author_with_force_push_requires_confirmation author_without_force_push_does_not_require_confirmation`
Expected: FAIL — no match arm for `Author` in `classify`

- [ ] **Step 3: Add risk classification**

In `src/risk.rs`, update the `classify` function. Add before the `_ =>` arm:

```rust
        CommandKind::Author { force, push, .. } if *force && *push => {
            high("author rewrite with force push rewrites history and pushes to remote")
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test cli author_with_force_push author_without_force_push`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/risk.rs tests/cli.rs tests/cli_cases/author.rs
git commit -m "feat(author): classify author --force --push as high risk"
```

---

### Task 3: Git operations — global config reading, log scanning, pushed detection

**Files:**
- Modify: `src/git/worktree.rs` (add new methods)

- [ ] **Step 1: Write the failing test**

Add to `tests/cli_cases/author.rs`:

```rust
#[test]
fn author_sets_local_config_when_no_commits_need_rewriting() {
    let repo = init_repo();
    // init_repo sets user.name = "Test User", user.email = "test@example.com"
    // The initial commit has author "Test User <test@example.com>"
    // We pass the same name/email, so no rewrite is needed — just config set.
    let output = run_agent(
        repo.path(),
        &["--json", "author", "--name", "Test User", "--email", "test@example.com"],
    );
    assert!(output.status.success(), "author should succeed: {:?}", output);

    let local_name = git_stdout(repo.path(), &["config", "--local", "user.name"]);
    assert_eq!(local_name.trim(), "Test User");
    let local_email = git_stdout(repo.path(), &["config", "--local", "user.email"]);
    assert_eq!(local_email.trim(), "test@example.com");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test cli author_sets_local_config_when_no_commits_need_rewriting`
Expected: FAIL — `run_author` not implemented

- [ ] **Step 3: Add git helper methods to worktree.rs**

Add these methods to `src/git/worktree.rs` inside the `impl GitExec` block:

```rust
    pub async fn global_user_name(&self) -> Result<String> {
        let output = Command::new("git")
            .args(["config", "--global", "user.name"])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("failed to read global user.name"));
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    pub async fn global_user_email(&self) -> Result<String> {
        let output = Command::new("git")
            .args(["config", "--global", "user.email"])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("failed to read global user.email"));
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    pub async fn set_local_user(&self, name: &str, email: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["config", "--local", "user.name", name])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("failed to set local user.name"));
        }
        let output = Command::new("git")
            .args(["config", "--local", "user.email", email])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("failed to set local user.email"));
        }
        Ok(())
    }

    /// Returns vec of (hash, author_name, author_email) from HEAD backwards.
    pub async fn log_authors(&self, limit: usize) -> Result<Vec<(String, String, String)>> {
        let output = Command::new("git")
            .args(["log", "--pretty=%H%n%an%n%ae", "-n", &limit.to_string()])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("failed to read commit log"));
        }
        let text = String::from_utf8(output.stdout)?;
        let lines: Vec<&str> = text.lines().collect();
        let mut result = Vec::new();
        for chunk in lines.chunks(3) {
            if chunk.len() == 3 {
                result.push((
                    chunk[0].trim().to_string(),
                    chunk[1].trim().to_string(),
                    chunk[2].trim().to_string(),
                ));
            }
        }
        Ok(result)
    }

    /// Check if a commit hash exists on the remote tracking branch.
    pub async fn is_commit_pushed(&self, hash: &str) -> Result<bool> {
        let output = Command::new("git")
            .args(["branch", "-r", "--contains", hash])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Ok(false);
        }
        let text = String::from_utf8(output.stdout)?;
        Ok(!text.trim().is_empty())
    }

    pub async fn rewrite_author(
        &self,
        count: usize,
        name: &str,
        email: &str,
    ) -> Result<()> {
        let author = format!("{name} <{email}>");
        let exec_cmd = format!("git commit --amend --author=\"{author}\" --no-edit");
        let output = Command::new("git")
            .args([
                "rebase",
                &format!("HEAD~{count}"),
                "--exec",
                &exec_cmd,
            ])
            .env("GIT_SEQUENCE_EDITOR", "true")
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Abort the rebase to restore original state
            let _ = Command::new("git")
                .args(["rebase", "--abort"])
                .current_dir(&self.cwd)
                .output()
                .await;
            return Err(anyhow!("author rewrite failed: {stderr}"));
        }
        Ok(())
    }

    pub async fn force_push_with_lease(&self) -> Result<()> {
        let output = Command::new("git")
            .args(["push", "--force-with-lease"])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("force push failed: {stderr}"));
        }
        Ok(())
    }
```

- [ ] **Step 4: Verify compilation**

Run: `cargo build`
Expected: Compiles (with warnings about unused methods, which is fine)

- [ ] **Step 5: Commit**

```bash
git add src/git/worktree.rs
git commit -m "feat(author): add git helper methods for author scanning and rewriting"
```

---

### Task 4: Core logic — `run_author` function

**Files:**
- Create: `src/commands/author.rs`
- Modify: `src/commands/mod.rs:1-4` (add `mod author`)

- [ ] **Step 1: Write the failing test**

Add to `tests/cli_cases/author.rs`:

```rust
#[test]
fn author_rewrites_consecutive_commits_with_global_author() {
    let repo = init_repo();
    // init_repo configures user.name="Test User", user.email="test@example.com"
    // and creates 1 initial commit with that author.

    // Create 2 more commits with the same (global-like) author
    fs::write(repo.path().join("a.txt"), "a\n").expect("write a");
    run_git(repo.path(), ["add", "a.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat: add a"]);

    fs::write(repo.path().join("b.txt"), "b\n").expect("write b");
    run_git(repo.path(), ["add", "b.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat: add b"]);

    // Now rewrite to a different author
    let output = run_agent(
        repo.path(),
        &["--json", "author", "--name", "Viking", "--email", "viking@example.com"],
    );
    assert!(output.status.success(), "author should succeed: {:?}", output);

    // All 3 commits (init + a + b) should now have the new author
    let log = git_stdout(repo.path(), &["log", "--pretty=%an <%ae>"]);
    for line in log.trim().lines() {
        assert_eq!(line.trim(), "Viking <viking@example.com>", "commit author mismatch: {line}");
    }

    // Local config should also be set
    let local_name = git_stdout(repo.path(), &["config", "--local", "user.name"]);
    assert_eq!(local_name.trim(), "Viking");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --test cli author_rewrites_consecutive_commits_with_global_author`
Expected: FAIL — `run_author` does not exist yet

- [ ] **Step 3: Create `src/commands/author.rs`**

```rust
use crate::events::Emitter;
use crate::git::{self, GitExec};
use anyhow::{Result, anyhow};
use serde_json::json;
use std::path::PathBuf;

pub(crate) struct AuthorRun {
    pub(crate) name: String,
    pub(crate) email: String,
    pub(crate) force: bool,
    pub(crate) push: bool,
}

pub(crate) async fn run_author(
    request: AuthorRun,
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    emitter: &mut Emitter,
) -> Result<()> {
    if request.push && !request.force {
        return Err(anyhow!("--push requires --force"));
    }

    let repo_ctx = repo.ok_or_else(|| anyhow!("author requires a git repository"))?;
    let git = GitExec::new(cwd.clone(), Some(repo_ctx.clone()));

    // Step 1: Read global git config
    let global_name = git.global_user_name().await?;
    let global_email = git.global_user_email().await?;

    // Step 2: Scan from HEAD backwards for consecutive commits matching global author
    let log = git.log_authors(500).await?;
    let mut rewrite_count = 0usize;
    for (_, name, email) in &log {
        if name == &global_name && email == &global_email {
            rewrite_count += 1;
        } else {
            break;
        }
    }

    // Step 3: Set project-level author config
    git.set_local_user(&request.name, &request.email).await?;

    if rewrite_count == 0 {
        emitter
            .emit(
                "tool_result",
                Some("done"),
                Some("author_set".to_string()),
                Some(json!({
                    "name": request.name,
                    "email": request.email,
                    "rewritten": 0,
                })),
            )
            .await?;
        if !emitter.json_mode() {
            println!(
                "\u{2713} Project author set: {} <{}>\n  No commits need rewriting.",
                request.name, request.email
            );
        }
        return Ok(());
    }

    // Step 4: Check if any commits to rewrite are already pushed
    let hashes_to_rewrite: Vec<&str> = log[..rewrite_count]
        .iter()
        .map(|(hash, _, _)| hash.as_str())
        .collect();
    let mut pushed_count = 0usize;
    for hash in &hashes_to_rewrite {
        if git.is_commit_pushed(hash).await? {
            pushed_count += 1;
        }
    }

    if pushed_count > 0 && !request.force {
        let msg = format!(
            "{rewrite_count} commits need rewriting, but {pushed_count} are already pushed to remote.\n  \
             Rerun with --force to rewrite, or --force --push to rewrite and push."
        );
        emitter
            .emit(
                "commandFailed",
                Some("exec"),
                Some(msg.clone()),
                Some(json!({
                    "rewrite_count": rewrite_count,
                    "pushed_count": pushed_count,
                })),
            )
            .await?;
        if !emitter.json_mode() {
            println!("\u{2717} {msg}");
        }
        return Err(anyhow!("commits already pushed; use --force"));
    }

    // Step 5: Rewrite
    emitter
        .emit(
            "phase_changed",
            Some("exec"),
            Some(format!("rewriting {rewrite_count} commits")),
            None,
        )
        .await?;

    git.rewrite_author(rewrite_count, &request.name, &request.email)
        .await?;

    // Step 6: Optional force push
    if request.push {
        emitter
            .emit(
                "phase_changed",
                Some("exec"),
                Some("force pushing to remote".to_string()),
                None,
            )
            .await?;
        git.force_push_with_lease().await?;
    }

    let old_author = format!("{global_name} <{global_email}>");
    let new_author = format!("{} <{}>", request.name, request.email);
    emitter
        .emit(
            "tool_result",
            Some("done"),
            Some("author_rewrite".to_string()),
            Some(json!({
                "name": request.name,
                "email": request.email,
                "rewritten": rewrite_count,
                "pushed": request.push,
            })),
        )
        .await?;
    if !emitter.json_mode() {
        println!(
            "\u{2713} Project author set: {new_author}\n  \
             Rewrote {rewrite_count} commits (HEAD~{}..HEAD)\n  \
             old: {old_author}\n  \
             new: {new_author}",
            rewrite_count - 1
        );
        if request.push {
            println!("  Force pushed to remote.");
        }
    }

    Ok(())
}
```

- [ ] **Step 4: Register the module in `src/commands/mod.rs`**

Add to `src/commands/mod.rs`:

```rust
pub(crate) mod author;
```

And add the pub use:

```rust
pub(crate) use author::{AuthorRun, run_author};
```

The complete file should be:

```rust
pub(crate) mod ai_tasks;
pub(crate) mod author;
pub(crate) mod branch;
pub(crate) mod commit;
pub(crate) mod merge_rebase;
```

- [ ] **Step 5: Verify compilation**

Run: `cargo build`
Expected: Compiles (with warning about unused `run_author`)

- [ ] **Step 6: Commit**

```bash
git add src/commands/author.rs src/commands/mod.rs
git commit -m "feat(author): implement run_author core logic"
```

---

### Task 5: Dispatch — Wire Author into dispatch.rs

**Files:**
- Modify: `src/app/dispatch.rs:1-4` (add import)
- Modify: `src/app/dispatch.rs:160-233` (add match arm)

- [ ] **Step 1: Run the existing failing test**

Run: `cargo test --test cli author_rewrites_consecutive_commits_with_global_author`
Expected: FAIL — `Author` not matched in `dispatch_command`

- [ ] **Step 2: Add the import**

In `src/app/dispatch.rs`, add to the imports at line 1-4:

```rust
use crate::commands::author::{AuthorRun, run_author};
```

- [ ] **Step 3: Add the match arm**

In `src/app/dispatch.rs`, inside the `match cli.command` block (after the `Rebase` arm), add:

```rust
        CommandKind::Author {
            name,
            email,
            force,
            push,
        } => {
            run_author(
                AuthorRun {
                    name,
                    email,
                    force,
                    push,
                },
                cwd.clone(),
                repo.clone(),
                emitter,
            )
            .await
        }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --test cli author_sets_local_config author_rewrites_consecutive author_with_force_push_requires author_without_force_push`
Expected: All PASS

- [ ] **Step 5: Commit**

```bash
git add src/app/dispatch.rs
git commit -m "feat(author): wire Author command into dispatch"
```

---

### Task 6: Edge case tests — push-without-force, mixed authors, empty repo

**Files:**
- Modify: `tests/cli_cases/author.rs` (add tests)

- [ ] **Step 1: Add push-without-force test**

Add to `tests/cli_cases/author.rs`:

```rust
#[test]
fn author_push_without_force_fails() {
    let repo = init_repo();
    let output = run_agent(
        repo.path(),
        &["--json", "author", "--name", "X", "--email", "x@x.com", "--push"],
    );
    assert!(!output.status.success(), "author --push without --force should fail");
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(
        stderr.contains("--push requires --force") || stdout.contains("--push requires --force"),
        "should mention --push requires --force"
    );
}
```

- [ ] **Step 2: Add mixed-author stop-at-boundary test**

```rust
#[test]
fn author_stops_rewriting_at_different_author_boundary() {
    let repo = init_repo();
    // init_repo: commit by "Test User <test@example.com>"

    // Commit by a different author
    run_git(repo.path(), ["config", "user.name", "Other Dev"]);
    run_git(repo.path(), ["config", "user.email", "other@dev.com"]);
    fs::write(repo.path().join("other.txt"), "other\n").expect("write other");
    run_git(repo.path(), ["add", "other.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat: other dev commit"]);

    // Switch back to original author for 2 more commits
    run_git(repo.path(), ["config", "user.name", "Test User"]);
    run_git(repo.path(), ["config", "user.email", "test@example.com"]);
    fs::write(repo.path().join("a.txt"), "a\n").expect("write a");
    run_git(repo.path(), ["add", "a.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat: add a"]);
    fs::write(repo.path().join("b.txt"), "b\n").expect("write b");
    run_git(repo.path(), ["add", "b.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat: add b"]);

    // Rewrite: should only rewrite the top 2 commits (a, b)
    let output = run_agent(
        repo.path(),
        &["--json", "author", "--name", "Viking", "--email", "viking@example.com"],
    );
    assert!(output.status.success(), "author should succeed: {:?}", output);

    let log = git_stdout(repo.path(), &["log", "--pretty=%an <%ae>", "-n", "4"]);
    let lines: Vec<&str> = log.trim().lines().collect();
    assert_eq!(lines[0].trim(), "Viking <viking@example.com>"); // HEAD (was b)
    assert_eq!(lines[1].trim(), "Viking <viking@example.com>"); // HEAD~1 (was a)
    assert_eq!(lines[2].trim(), "Other Dev <other@dev.com>");    // HEAD~2 (unchanged)
    assert_eq!(lines[3].trim(), "Test User <test@example.com>"); // HEAD~3 (unchanged — init)
}
```

- [ ] **Step 3: Run all author tests**

Run: `cargo test --test cli author_`
Expected: All PASS

- [ ] **Step 4: Commit**

```bash
git add tests/cli_cases/author.rs
git commit -m "test(author): add edge case tests for push validation and boundary detection"
```

---

### Task 7: Guardrails and help output test update

**Files:**
- Modify: `tests/cli.rs:109-135` (update help test)

- [ ] **Step 1: Update root help test**

In `tests/cli.rs`, add this assertion inside `root_help_lists_global_flags_and_agent_commands`:

```rust
    assert!(stdout.contains("Set project-level commit author"));
```

- [ ] **Step 2: Run guardrails and help tests**

Run: `cargo test --test cli root_help_lists && cargo test --test guardrails`
Expected: All PASS

- [ ] **Step 3: Commit**

```bash
git add tests/cli.rs
git commit -m "test(author): update help output assertion for author command"
```

---

### Task 8: Clippy and final verification

**Files:** None (verification only)

- [ ] **Step 1: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings or errors

- [ ] **Step 2: Run full test suite**

Run: `cargo test`
Expected: All tests PASS

- [ ] **Step 3: Run format check**

Run: `cargo fmt --check`
Expected: No formatting issues

- [ ] **Step 4: Final commit if any fixes needed**

If clippy or fmt required changes:

```bash
git add -u
git commit -m "fix(author): address clippy and formatting issues"
```
