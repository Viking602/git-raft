use super::*;

/// Write a .gitconfig in the fake HOME so `git config --global` works in tests.
fn setup_global_gitconfig(repo: &TempDir) {
    let home = repo.path().with_extension("home");
    fs::create_dir_all(&home).expect("create home");
    fs::write(
        home.join(".gitconfig"),
        "[user]\n\tname = Test User\n\temail = test@example.com\n",
    )
    .expect("write global gitconfig");
}

fn setup_remote(repo: &TempDir) -> TempDir {
    let remote = TempDir::new().expect("create remote");
    run_git(remote.path(), ["init", "--bare", "--initial-branch=main"]);
    run_git(
        repo.path(),
        [
            "remote",
            "add",
            "origin",
            remote.path().to_str().expect("remote path"),
        ],
    );
    run_git(repo.path(), ["push", "-u", "origin", "main"]);
    remote
}

#[test]
fn author_help_lists_name_email_and_force_flags() {
    let stdout = help_output(&["author", "--help"]);
    assert!(stdout.contains("Set project-level commit author"));
    assert!(stdout.contains("--name"));
    assert!(stdout.contains("--email"));
    assert!(stdout.contains("--force"));
    assert!(stdout.contains("--push"));
}

#[test]
fn author_with_force_push_requires_confirmation() {
    let repo = init_repo();
    let output = run_agent(
        repo.path(),
        &[
            "--json", "author", "--name", "Test", "--email", "t@t.com", "--force", "--push",
        ],
    );
    assert!(
        !output.status.success(),
        "author --force --push should require --yes"
    );
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

#[test]
fn author_sets_local_config_when_no_commits_need_rewriting() {
    let repo = init_repo();
    setup_global_gitconfig(&repo);
    // init_repo sets user.name = "Test User", user.email = "test@example.com"
    // The initial commit has author "Test User <test@example.com>"
    // We pass the same name/email, so no rewrite is needed — just config set.
    let output = run_agent(
        repo.path(),
        &[
            "--json",
            "author",
            "--name",
            "Test User",
            "--email",
            "test@example.com",
        ],
    );
    assert!(
        output.status.success(),
        "author should succeed: {:?}",
        output
    );

    let local_name = git_stdout(repo.path(), &["config", "--local", "user.name"]);
    assert_eq!(local_name.trim(), "Test User");
    let local_email = git_stdout(repo.path(), &["config", "--local", "user.email"]);
    assert_eq!(local_email.trim(), "test@example.com");
}

#[test]
fn author_rewrites_consecutive_commits_with_global_author() {
    let repo = init_repo();
    setup_global_gitconfig(&repo);

    fs::write(repo.path().join("a.txt"), "a\n").expect("write a");
    run_git(repo.path(), ["add", "a.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat: add a"]);

    fs::write(repo.path().join("b.txt"), "b\n").expect("write b");
    run_git(repo.path(), ["add", "b.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat: add b"]);

    let output = run_agent(
        repo.path(),
        &[
            "--json",
            "author",
            "--name",
            "Viking",
            "--email",
            "viking@example.com",
        ],
    );
    assert!(
        output.status.success(),
        "author should succeed: {:?}",
        output
    );

    let log = git_stdout(repo.path(), &["log", "--pretty=%an <%ae>"]);
    for line in log.trim().lines() {
        assert_eq!(
            line.trim(),
            "Viking <viking@example.com>",
            "commit author mismatch: {line}"
        );
    }

    let local_name = git_stdout(repo.path(), &["config", "--local", "user.name"]);
    assert_eq!(local_name.trim(), "Viking");
}

#[test]
fn author_uses_local_author_when_global_config_is_missing() {
    let repo = init_repo();

    let output = run_agent(
        repo.path(),
        &[
            "--json",
            "author",
            "--name",
            "Viking",
            "--email",
            "viking@example.com",
        ],
    );
    assert!(
        output.status.success(),
        "author should fall back to local config: {:?}",
        output
    );

    let log = git_stdout(repo.path(), &["log", "--pretty=%an <%ae>", "-n", "1"]);
    assert_eq!(log.trim(), "Viking <viking@example.com>");
}

#[test]
fn author_prefers_local_author_over_global_when_planning_rewrite() {
    let repo = init_repo();
    setup_global_gitconfig(&repo);

    run_git(repo.path(), ["config", "user.name", "Repo Dev"]);
    run_git(repo.path(), ["config", "user.email", "repo@example.com"]);

    fs::write(repo.path().join("a.txt"), "a\n").expect("write a");
    run_git(repo.path(), ["add", "a.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat: add a"]);

    fs::write(repo.path().join("b.txt"), "b\n").expect("write b");
    run_git(repo.path(), ["add", "b.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat: add b"]);

    let output = run_agent(
        repo.path(),
        &[
            "--json",
            "author",
            "--name",
            "Viking",
            "--email",
            "viking@example.com",
        ],
    );
    assert!(
        output.status.success(),
        "author should rewrite commits made with the repo-local author: {:?}",
        output
    );

    let log = git_stdout(repo.path(), &["log", "--pretty=%an <%ae>", "-n", "3"]);
    let lines: Vec<&str> = log.trim().lines().collect();
    assert_eq!(lines[0].trim(), "Viking <viking@example.com>");
    assert_eq!(lines[1].trim(), "Viking <viking@example.com>");
    assert_eq!(lines[2].trim(), "Test User <test@example.com>");
}

#[test]
fn author_does_not_change_local_config_when_force_is_required() {
    let repo = init_repo();
    setup_global_gitconfig(&repo);
    let _remote = setup_remote(&repo);

    let output = run_agent(
        repo.path(),
        &[
            "--json",
            "author",
            "--name",
            "Viking",
            "--email",
            "viking@example.com",
        ],
    );
    assert!(
        !output.status.success(),
        "author should stop before rewriting pushed commits"
    );

    let local_name = git_stdout(repo.path(), &["config", "--local", "user.name"]);
    assert_eq!(local_name.trim(), "Test User");
    let local_email = git_stdout(repo.path(), &["config", "--local", "user.email"]);
    assert_eq!(local_email.trim(), "test@example.com");
}

#[test]
fn author_push_without_force_fails() {
    let repo = init_repo();
    let output = run_agent(
        repo.path(),
        &[
            "--json", "author", "--name", "X", "--email", "x@x.com", "--push",
        ],
    );
    assert!(
        !output.status.success(),
        "author --push without --force should fail"
    );
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(
        stderr.contains("--push requires --force") || stdout.contains("--push requires --force"),
        "should mention --push requires --force"
    );
}

#[test]
fn author_stops_rewriting_at_different_author_boundary() {
    let repo = init_repo();
    setup_global_gitconfig(&repo);

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
        &[
            "--json",
            "author",
            "--name",
            "Viking",
            "--email",
            "viking@example.com",
        ],
    );
    assert!(
        output.status.success(),
        "author should succeed: {:?}",
        output
    );

    let log = git_stdout(repo.path(), &["log", "--pretty=%an <%ae>", "-n", "4"]);
    let lines: Vec<&str> = log.trim().lines().collect();
    assert_eq!(lines[0].trim(), "Viking <viking@example.com>"); // HEAD (was b)
    assert_eq!(lines[1].trim(), "Viking <viking@example.com>"); // HEAD~1 (was a)
    assert_eq!(lines[2].trim(), "Other Dev <other@dev.com>"); // HEAD~2 (unchanged)
    assert_eq!(lines[3].trim(), "Test User <test@example.com>"); // HEAD~3 (unchanged)
}

#[test]
fn author_preserves_quotes_in_name_during_rewrite() {
    let repo = init_repo();
    setup_global_gitconfig(&repo);

    let output = run_agent(
        repo.path(),
        &[
            "--json",
            "author",
            "--name",
            "John \"JD\" Doe",
            "--email",
            "john@example.com",
        ],
    );
    assert!(
        output.status.success(),
        "author should preserve quoted names: {:?}",
        output
    );

    let log = git_stdout(repo.path(), &["log", "--pretty=%an <%ae>", "-n", "1"]);
    assert_eq!(log.trim(), "John \"JD\" Doe <john@example.com>");
}
