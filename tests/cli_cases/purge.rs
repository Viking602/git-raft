use super::*;

#[test]
fn purge_help_lists_paths_and_force_flags() {
    let stdout = help_output(&["purge", "--help"]);
    assert!(stdout.contains("Remove files from the branch and rewrite history"));
    assert!(stdout.contains("PATHS"));
    assert!(stdout.contains("--force"));
    assert!(stdout.contains("--push"));
}

#[test]
fn purge_with_force_push_requires_confirmation() {
    let repo = init_repo();
    let output = run_agent(
        repo.path(),
        &["--json", "purge", "--force", "--push", "README.md"],
    );
    assert!(
        !output.status.success(),
        "purge --force --push should require --yes"
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("\"event_type\":\"awaiting_confirmation\""));
}

#[test]
fn purge_without_force_push_does_not_require_confirmation() {
    let repo = init_repo();
    let output = run_agent(repo.path(), &["--json", "purge", "README.md"]);
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(!stdout.contains("\"event_type\":\"awaiting_confirmation\""));
}

#[test]
fn purge_removes_file_from_history_but_keeps_on_disk() {
    let repo = init_repo();
    fs::write(repo.path().join("secret.txt"), "password123\n").expect("write secret");
    run_git(repo.path(), ["add", "secret.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat: add secret"]);

    assert!(repo.path().join("secret.txt").exists());

    let output = run_agent(repo.path(), &["purge", "secret.txt"]);
    assert!(output.status.success(), "purge failed: {:?}", output);

    // File should still exist on disk
    assert!(repo.path().join("secret.txt").exists());

    // File should be gone from history
    let log = git_stdout(
        repo.path(),
        &["log", "--all", "--pretty=format:", "--name-only"],
    );
    assert!(
        !log.contains("secret.txt"),
        "secret.txt should be removed from history"
    );

    // .gitignore should contain the path
    let gitignore = fs::read_to_string(repo.path().join(".gitignore")).expect("read gitignore");
    assert!(
        gitignore.contains("secret.txt"),
        ".gitignore should contain secret.txt"
    );
}

#[test]
fn purge_removes_directory_from_history_and_adds_to_gitignore() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("data")).expect("mkdir data");
    fs::write(repo.path().join("data/a.txt"), "a\n").expect("write a");
    fs::write(repo.path().join("data/b.txt"), "b\n").expect("write b");
    run_git(repo.path(), ["add", "data/"]);
    run_git(repo.path(), ["commit", "-m", "feat: add data dir"]);

    let output = run_agent(repo.path(), &["purge", "data"]);
    assert!(output.status.success(), "purge dir failed: {:?}", output);

    // Directory should still exist on disk
    assert!(repo.path().join("data/a.txt").exists());

    // But gone from history
    let log = git_stdout(
        repo.path(),
        &["log", "--all", "--pretty=format:", "--name-only"],
    );
    assert!(!log.contains("data/a.txt"));
    assert!(!log.contains("data/b.txt"));

    // .gitignore should contain the dir
    let gitignore = fs::read_to_string(repo.path().join(".gitignore")).expect("read gitignore");
    assert!(
        gitignore.contains("/data/"),
        ".gitignore should contain /data/"
    );
}

#[test]
fn purge_handles_nonexistent_path_gracefully() {
    let repo = init_repo();
    let output = run_agent(repo.path(), &["--json", "purge", "nonexistent.txt"]);
    assert!(
        output.status.success(),
        "purge should succeed with noop for missing path: {:?}",
        output
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("purge_noop"));
}

#[test]
fn purge_push_without_force_fails() {
    let repo = init_repo();
    let output = run_agent(repo.path(), &["purge", "--push", "README.md"]);
    assert!(
        !output.status.success(),
        "purge --push without --force should fail"
    );
    let stderr = String::from_utf8(output.stderr).expect("utf8");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(
        stderr.contains("--push requires --force") || stdout.contains("--push requires --force"),
        "should mention --push requires --force"
    );
}

#[test]
fn purge_multiple_paths() {
    let repo = init_repo();
    fs::write(repo.path().join("a.txt"), "a\n").expect("write a");
    fs::write(repo.path().join("b.txt"), "b\n").expect("write b");
    run_git(repo.path(), ["add", "a.txt", "b.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat: add a and b"]);

    let output = run_agent(repo.path(), &["purge", "a.txt", "b.txt"]);
    assert!(
        output.status.success(),
        "purge multiple paths failed: {:?}",
        output
    );

    // Files remain on disk
    assert!(repo.path().join("a.txt").exists());
    assert!(repo.path().join("b.txt").exists());

    // Gone from history
    let log = git_stdout(
        repo.path(),
        &["log", "--all", "--pretty=format:", "--name-only"],
    );
    assert!(!log.contains("a.txt"));
    assert!(!log.contains("b.txt"));
}

#[test]
fn purge_stops_when_pushed_and_no_force() {
    let repo = init_repo();
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

    fs::write(repo.path().join("secret.txt"), "password\n").expect("write secret");
    run_git(repo.path(), ["add", "secret.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat: add secret"]);
    run_git(repo.path(), ["push"]);

    let output = run_agent(repo.path(), &["purge", "secret.txt"]);
    assert!(
        !output.status.success(),
        "purge should stop when commits are pushed without --force"
    );
    assert!(repo.path().join("secret.txt").exists());
}

#[test]
fn purge_works_with_unstaged_changes() {
    let repo = init_repo();
    fs::write(repo.path().join("secret.txt"), "password\n").expect("write secret");
    run_git(repo.path(), ["add", "secret.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat: add secret"]);

    // Create unstaged changes
    fs::write(repo.path().join("README.md"), "modified\n").expect("modify readme");

    let output = run_agent(repo.path(), &["purge", "secret.txt"]);
    assert!(
        output.status.success(),
        "purge should work with unstaged changes: {:?}",
        output
    );

    // Unstaged change should be preserved
    let readme = fs::read_to_string(repo.path().join("README.md")).expect("read readme");
    assert_eq!(readme, "modified\n");

    // secret.txt gone from history
    let log = git_stdout(
        repo.path(),
        &["log", "--all", "--pretty=format:", "--name-only"],
    );
    assert!(!log.contains("secret.txt"));
}
