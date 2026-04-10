use std::fs;
use std::process::Command as StdCommand;
use tempfile::TempDir;
mod support;

use support::*;

fn help_output(args: &[&str]) -> String {
    let dir = TempDir::new().expect("help dir");
    let output = run_agent(dir.path(), args);
    assert!(output.status.success(), "help failed: {:?}", output);
    String::from_utf8(output.stdout).expect("utf8")
}

fn git_stdout(repo: &std::path::Path, args: &[&str]) -> String {
    let output = StdCommand::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git");
    assert!(output.status.success(), "git command failed: {:?}", output);
    String::from_utf8(output.stdout).expect("utf8")
}

fn init_conflict_repo() -> TempDir {
    let repo = init_repo();
    fs::write(repo.path().join("conflict.txt"), "base\n").expect("write conflict file");
    run_git(repo.path(), ["add", "conflict.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat(conflict): add fixture"]);
    run_git(repo.path(), ["checkout", "-b", "feature"]);
    fs::write(repo.path().join("conflict.txt"), "feature\n").expect("write feature");
    run_git(repo.path(), ["add", "conflict.txt"]);
    run_git(
        repo.path(),
        ["commit", "-m", "feat(conflict): change feature"],
    );
    run_git(repo.path(), ["checkout", "main"]);
    fs::write(repo.path().join("conflict.txt"), "main\n").expect("write main");
    run_git(repo.path(), ["add", "conflict.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat(conflict): change main"]);
    repo
}

fn validation_script(required: &[&str], forbidden: &[&str]) -> String {
    if cfg!(windows) {
        let mut script = String::from("$content = Get-Content \"conflict.txt\" -Raw\n");
        for value in required {
            script.push_str(&format!(
                "if (-not $content.Contains('{}')) {{ Write-Error 'missing {}'; exit 1 }}\n",
                value.replace('\'', "''"),
                value.replace('\'', "''")
            ));
        }
        for value in forbidden {
            script.push_str(&format!(
                "if ($content.Contains('{}')) {{ Write-Error 'forbidden {}'; exit 1 }}\n",
                value.replace('\'', "''"),
                value.replace('\'', "''")
            ));
        }
        script.push_str("exit 0\n");
        script
    } else {
        let mut script = String::from("#!/bin/sh\ncontent=$(cat conflict.txt)\n");
        for value in required {
            script.push_str(&format!(
                "printf '%s' \"$content\" | grep -Fq {} || exit 1\n",
                format!("{value:?}")
            ));
        }
        for value in forbidden {
            script.push_str(&format!(
                "if printf '%s' \"$content\" | grep -Fq {}; then exit 1; fi\n",
                format!("{value:?}")
            ));
        }
        script.push_str("exit 0\n");
        script
    }
}

fn init_binary_conflict_repo() -> TempDir {
    let repo = init_repo();
    fs::write(repo.path().join("conflict.bin"), vec![0x66, 0x6f, 0x80])
        .expect("write binary fixture");
    run_git(repo.path(), ["add", "conflict.bin"]);
    run_git(
        repo.path(),
        ["commit", "-m", "feat(conflict): add binary fixture"],
    );
    run_git(repo.path(), ["checkout", "-b", "feature"]);
    fs::write(repo.path().join("conflict.bin"), vec![0x66, 0x6f, 0x81])
        .expect("write binary feature");
    run_git(repo.path(), ["add", "conflict.bin"]);
    run_git(
        repo.path(),
        ["commit", "-m", "feat(conflict): change binary feature"],
    );
    run_git(repo.path(), ["checkout", "main"]);
    fs::write(repo.path().join("conflict.bin"), vec![0x66, 0x6f, 0x82]).expect("write binary main");
    run_git(repo.path(), ["add", "conflict.bin"]);
    run_git(
        repo.path(),
        ["commit", "-m", "feat(conflict): change binary main"],
    );
    repo
}

#[test]
fn root_help_lists_global_flags_and_agent_commands() {
    let stdout = help_output(&["--help"]);
    assert!(stdout.contains("Emit newline-delimited JSON events"));
    assert!(stdout.contains("Skip confirmation prompts for high-risk operations"));
    assert!(stdout.contains("Ask the AI planner to group changes and create commits"));
    assert!(stdout.contains("Create and switch to a new branch from a commit"));
    assert!(stdout.contains("Run git merge and optionally ask AI to resolve conflicts"));
    assert!(stdout.contains("Run git rebase and optionally ask AI to resolve conflicts"));
    assert!(stdout.contains("Set project-level commit author"));
    assert!(stdout.contains("Remove files from the branch and rewrite history to erase them"));
    assert!(!stdout.contains("Send a free-form prompt to the configured AI provider"));

    assert!(!stdout.contains("Pass arguments through to git status"));
    assert!(!stdout.contains("Pass arguments through to git diff"));
    assert!(!stdout.contains("Pass arguments through to git add"));
    assert!(!stdout.contains("Pass arguments through to git branch"));
    assert!(!stdout.contains("Pass arguments through to git switch"));
    assert!(!stdout.contains("Fetch remotes and pull the current branch"));
    assert!(!stdout.contains("Pass arguments through to git stash"));
    assert!(!stdout.contains("Pass arguments through to git log"));
    assert!(!stdout.contains("Create default git-raft config files"));
    assert!(!stdout.contains("Reset the working tree to the backup ref saved for a previous run"));
    assert!(!stdout.contains("List saved runs for the current repository"));
    assert!(!stdout.contains("Show the saved event stream for a previous run"));
    assert!(!stdout.contains("Report repository, Git, and provider configuration status"));
    assert!(!stdout.contains("Show, read, or write git-raft configuration values"));
    assert!(!stdout.contains("Generate or list commit scope candidates"));
}

#[test]
fn commit_help_lists_planner_flags_and_language_values() {
    let stdout = help_output(&["commit", "--help"]);
    assert!(stdout.contains("Usage: git-raft commit [OPTIONS] [ARGS]..."));
    assert!(stdout.contains("--json"));
    assert!(stdout.contains("Print the planned commit groups without creating commits"));
    assert!(stdout.contains("Preview the planned commit execution without creating commits"));
    assert!(stdout.contains("--yes"));
    assert!(stdout.contains("Extra guidance passed to the AI commit planner"));
    assert!(stdout.contains("Override the configured commit message language for this run"));
    assert!(stdout.contains("--lang <LANGUAGE>"));
    assert!(!stdout.contains("--language"));
    assert!(stdout.contains("currently ignored by the commit planner"));
    assert!(stdout.contains("Possible values:"));
    assert!(stdout.contains("Generate commit messages in English"));
    assert!(stdout.contains("Generate commit messages in Chinese"));
    assert!(stdout.contains("-h, --help"));
}

#[test]
fn merge_help_lists_target_flag_and_passthrough_args() {
    let stdout = help_output(&["merge", "--help"]);
    assert!(stdout.contains("Branch, commit, or ref to merge into the current branch"));
    assert!(stdout.contains("Try AI conflict resolution when merge stops on conflicts"));
    assert!(stdout.contains("Extra arguments passed to git merge"));
}

#[test]
fn branch_help_lists_name_and_commit_arguments() {
    let stdout = help_output(&["branch", "--help"]);
    assert!(stdout.contains("Create and switch to a new branch from a commit"));
    assert!(stdout.contains("New branch name"));
    assert!(stdout.contains("Commit, short SHA, or ref to branch from"));
}

#[test]
fn removed_passthrough_command_is_rejected() {
    let repo = init_repo();
    let output = run_agent(repo.path(), &["status"]);
    assert!(
        !output.status.success(),
        "status should no longer be accepted: {:?}",
        output
    );

    let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
    assert!(stderr.contains("unrecognized subcommand"));
    assert!(stderr.contains("status"));

    let ask_output = run_agent(repo.path(), &["ask", "summarize", "repo"]);
    assert!(
        !ask_output.status.success(),
        "ask should no longer be accepted: {:?}",
        ask_output
    );
    let ask_stderr = String::from_utf8(ask_output.stderr).expect("utf8 stderr");
    assert!(ask_stderr.contains("unrecognized subcommand"));
    assert!(ask_stderr.contains("ask"));
}

#[test]
fn branch_creates_and_switches_from_short_commit_id() {
    let repo = init_repo();
    let target = git_stdout(repo.path(), &["rev-parse", "HEAD"]);
    std::fs::write(repo.path().join("hotfix.txt"), "hotfix\n").expect("write hotfix");
    run_git(repo.path(), ["add", "hotfix.txt"]);
    run_git(repo.path(), ["commit", "-m", "fix: add hotfix file"]);

    let short_target = target.trim()[..7].to_string();
    let output = run_agent(repo.path(), &["branch", "hotfix", &short_target]);
    assert!(output.status.success(), "branch failed: {:?}", output);

    let branch = git_stdout(repo.path(), &["branch", "--show-current"]);
    assert_eq!(branch.trim(), "hotfix");

    let head = git_stdout(repo.path(), &["rev-parse", "HEAD"]);
    assert_eq!(head.trim(), target.trim());
}

#[test]
fn branch_creates_and_switches_from_full_commit_id() {
    let repo = init_repo();
    let target = git_stdout(repo.path(), &["rev-parse", "HEAD"]);
    std::fs::write(repo.path().join("main.txt"), "main\n").expect("write main");
    run_git(repo.path(), ["add", "main.txt"]);
    run_git(repo.path(), ["commit", "-m", "feat: advance main"]);

    let output = run_agent(repo.path(), &["branch", "release-fix", target.trim()]);
    assert!(output.status.success(), "branch failed: {:?}", output);

    let branch = git_stdout(repo.path(), &["branch", "--show-current"]);
    assert_eq!(branch.trim(), "release-fix");

    let head = git_stdout(repo.path(), &["rev-parse", "HEAD"]);
    assert_eq!(head.trim(), target.trim());
}

#[test]
fn merge_requires_confirmation_without_yes() {
    let repo = init_repo();
    let output = run_agent(repo.path(), &["--json", "merge", "feature/demo"]);
    assert!(!output.status.success(), "merge unexpectedly succeeded");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("\"event_type\":\"awaiting_confirmation\""));
}

#[test]
fn before_command_hook_receives_camel_case_payload_for_agent_command() {
    let repo = init_repo();
    let payload_path = ".config/git-raft/hook-payload.json";
    let hook = write_external_hook(repo.path(), "capture", payload_path, None);
    write_ai_repo_config(
        repo.path(),
        "http://127.0.0.1:9",
        &external_hook_toml("beforeCommand", &hook),
    );

    let output = run_agent(repo.path(), &["--yes", "merge", "missing-branch"]);
    assert!(!output.status.success(), "merge should fail: {:?}", output);

    let payload = fs::read_to_string(repo.path().join(payload_path)).expect("payload");
    assert!(payload.contains("\"event\":\"beforeCommand\""));
    assert!(payload.contains("\"command\":\"merge\""));
    assert!(payload.contains("\"repoRoot\""));
    assert!(payload.contains("\"timestampMs\""));
    assert!(payload.contains("\"gitSnapshot\""));
}

#[path = "cli_cases/commit.rs"]
mod commit_cases;

#[path = "cli_cases/author.rs"]
mod author_cases;

#[path = "cli_cases/purge.rs"]
mod purge_cases;

#[test]
fn before_ai_request_hook_can_block_request() {
    let repo = init_repo();
    let payload_path = ".config/git-raft/before-ai-request.json";
    let hook = write_external_hook(
        repo.path(),
        "block-ai",
        payload_path,
        Some(r#"{"blocked":true,"reason":"blocked before ai request"}"#),
    );
    write_commit_ai_repo_config(
        repo.path(),
        "http://127.0.0.1:9",
        "",
        &external_hook_toml("beforeAiRequest", &hook),
    );

    let output = run_agent_with_env(
        repo.path(),
        &["--json", "commit", "--plan"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        !output.status.success(),
        "commit unexpectedly succeeded: {:?}",
        output
    );

    let payload = fs::read_to_string(repo.path().join(payload_path)).expect("hook payload");
    assert!(payload.contains("\"event\":\"beforeAiRequest\""));
    assert!(payload.contains("\"command\":\"commit\""));
    assert!(payload.contains("\"agentTask\":\"plan_commit\""));
    assert!(payload.contains("\"agentRequestSummary\""));

    let run_dir = latest_run_dir(repo.path());
    assert!(
        !run_dir.join("ai-request.json").exists(),
        "blocked request should not persist ai-request.json"
    );
}

#[test]
fn before_patch_apply_hook_blocks_apply_and_keeps_patch_json() {
    let repo = init_conflict_repo();

    let payload_path = ".config/git-raft/before-patch-apply.json";
    let hook = write_external_hook(
        repo.path(),
        "block-patch",
        payload_path,
        Some(r#"{"blocked":true,"reason":"blocked before patch apply"}"#),
    );
    let verify = write_repo_command_script(
        repo.path(),
        "verify-before-patch-apply",
        &validation_script(&["main", "feature", "resolved"], &["<<<<<<<"]),
    );
    let server = MockAiServer::start(vec![ai_patch_response(
        "conflict.txt",
        "main\nfeature\nresolved\n",
    )]);
    write_merge_ai_repo_config(
        repo.path(),
        server.url(),
        &merge_verification_toml(1, &[verify]),
        &external_hook_toml("beforePatchApply", &hook),
    );

    let output = run_agent_with_env(
        repo.path(),
        &["--json", "--yes", "merge", "feature"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        !output.status.success(),
        "merge should still stop after blocked patch apply"
    );

    let run_dir = latest_run_dir(repo.path());
    assert!(run_dir.join("patch.json").exists(), "missing patch.json");
    let payload = fs::read_to_string(repo.path().join(payload_path)).expect("hook payload");
    assert!(payload.contains("\"event\":\"beforePatchApply\""));
    assert!(payload.contains("\"agentTask\":\"resolve_conflicts\""));
    assert!(payload.contains("\"patchConfidence\":0.95"));

    let conflict = fs::read_to_string(repo.path().join("conflict.txt")).expect("conflict file");
    assert!(
        conflict.contains("<<<<<<<"),
        "patch should not have been applied"
    );
}

#[test]
fn merge_keeps_candidate_for_review_when_verification_is_not_configured() {
    let repo = init_conflict_repo();
    let server = MockAiServer::start(vec![ai_patch_response("conflict.txt", "main\nfeature\n")]);
    write_ai_repo_config(repo.path(), server.url(), "");

    let output = run_agent_with_env(
        repo.path(),
        &["--json", "--yes", "merge", "feature"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        !output.status.success(),
        "merge should stop for review without verification config"
    );

    let run_dir = latest_run_dir(repo.path());
    assert!(run_dir.join("patch.json").exists(), "missing patch.json");
    assert!(
        run_dir.join("validation.json").exists(),
        "missing validation.json"
    );

    let conflict = fs::read_to_string(repo.path().join("conflict.txt")).expect("conflict file");
    assert!(
        conflict.contains("<<<<<<<"),
        "real worktree should remain conflicted without verification config"
    );

    let validation = fs::read_to_string(run_dir.join("validation.json")).expect("validation json");
    assert!(validation.contains("verification"), "{validation}");
}

#[test]
fn merge_retries_after_retention_failure_and_applies_second_candidate() {
    let repo = init_conflict_repo();
    let payload_path = ".config/git-raft/before-patch-apply-retry.json";
    let hook = write_external_hook(repo.path(), "capture-retry", payload_path, None);
    let verify = write_repo_command_script(
        repo.path(),
        "verify-merge-retry",
        &validation_script(&["main", "feature"], &["<<<<<<<"]),
    );
    let server = MockAiServer::start(vec![
        ai_patch_response("conflict.txt", "main\n"),
        ai_patch_response("conflict.txt", "main\nfeature\n"),
    ]);
    write_merge_ai_repo_config(
        repo.path(),
        server.url(),
        &merge_verification_toml(1, &[verify]),
        &external_hook_toml("beforePatchApply", &hook),
    );

    let output = run_agent_with_env(
        repo.path(),
        &["--json", "--yes", "merge", "feature"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        output.status.success(),
        "merge should succeed after repair: {output:?}"
    );

    let content = fs::read_to_string(repo.path().join("conflict.txt")).expect("resolved file");
    assert_eq!(content, "main\nfeature\n");

    let second_request = nth_ai_user_request(&server, 1);
    assert_eq!(second_request["user_payload"]["attempt"], 2);
    assert!(
        second_request["user_payload"]
            .get("repair_context")
            .is_some(),
        "second attempt should include repair context: {second_request}"
    );

    let payload = fs::read_to_string(repo.path().join(payload_path)).expect("hook payload");
    assert!(payload.contains("\"attempt\":2"), "{payload}");
    assert!(payload.contains("\"validationPassed\":true"), "{payload}");
}

#[test]
fn merge_stops_after_second_validation_failure_without_applying_candidate() {
    let repo = init_conflict_repo();
    let verify = write_repo_command_script(
        repo.path(),
        "verify-merge-fail",
        &validation_script(&["ready"], &["<<<<<<<"]),
    );
    let server = MockAiServer::start(vec![
        ai_patch_response("conflict.txt", "main\nfeature\n"),
        ai_patch_response("conflict.txt", "main\nfeature\n"),
    ]);
    write_merge_ai_repo_config(
        repo.path(),
        server.url(),
        &merge_verification_toml(1, &[verify]),
        "",
    );

    let output = run_agent_with_env(
        repo.path(),
        &["--json", "--yes", "merge", "feature"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        !output.status.success(),
        "merge should stop after second validation failure"
    );

    let conflict = fs::read_to_string(repo.path().join("conflict.txt")).expect("conflict file");
    assert!(
        conflict.contains("<<<<<<<"),
        "real worktree should stay conflicted after validation failures"
    );

    let run_dir = latest_run_dir(repo.path());
    let validation = fs::read_to_string(run_dir.join("validation.json")).expect("validation json");
    let validation_json: serde_json::Value =
        serde_json::from_str(&validation).expect("parse validation json");
    assert_eq!(validation_json["attempts"][1]["attempt"], 2);
    assert_eq!(validation_json["attempts"][1]["validationPassed"], false);
}

#[test]
fn merge_rejects_non_utf8_conflict_files_before_requesting_ai() {
    let repo = init_binary_conflict_repo();
    write_merge_ai_repo_config(
        repo.path(),
        "http://127.0.0.1:9",
        &merge_verification_toml(1, &[]),
        "",
    );

    let output = run_agent_with_env(
        repo.path(),
        &["--json", "--yes", "merge", "feature"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        !output.status.success(),
        "merge should stop when conflicted files are not UTF-8 text"
    );

    let run_dir = latest_run_dir(repo.path());
    let validation = fs::read_to_string(run_dir.join("validation.json")).expect("validation json");
    assert!(validation.contains("decodable text"), "{validation}");
}

#[test]
fn merge_request_uses_resolve_conflicts_tool() {
    let repo = init_conflict_repo();
    let server = MockAiServer::start(vec![ai_patch_response("conflict.txt", "main\nfeature\n")]);
    let verify = write_repo_command_script(
        repo.path(),
        "verify-tool-request",
        &validation_script(&["main", "feature"], &["<<<<<<<"]),
    );
    write_merge_ai_repo_config(
        repo.path(),
        server.url(),
        &merge_verification_toml(1, &[verify]),
        "",
    );

    let output = run_agent_with_env(
        repo.path(),
        &["--json", "--yes", "merge", "feature"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(output.status.success(), "merge failed: {:?}", output);

    let request = first_ai_provider_request(&server);
    assert_eq!(request["tool_choice"]["type"], "function");
    assert_eq!(
        request["tool_choice"]["function"]["name"],
        "resolve_conflicts"
    );
    assert_eq!(request["tools"][0]["type"], "function");
    assert_eq!(request["tools"][0]["function"]["name"], "resolve_conflicts");
    assert_eq!(request["temperature"], 0.0);
}

#[test]
fn conflict_request_prompt_requires_verbatim_unique_content_retention() {
    let repo = init_conflict_repo();
    write_merge_ai_repo_config(repo.path(), "http://127.0.0.1:9", "", "");

    let output = run_agent_with_env(
        repo.path(),
        &["--json", "--yes", "merge", "feature"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        !output.status.success(),
        "merge should fail when provider is unreachable: {:?}",
        output
    );

    let run_dir = latest_run_dir(repo.path());
    let request = fs::read_to_string(run_dir.join("ai-request.json")).expect("ai request");
    assert!(
        request.contains("Keep every unique line from ours and theirs in the resolved file text"),
        "{request}"
    );
    assert!(
        request.contains("When you combine behavior, keep the original unique lines verbatim and add code around them"),
        "{request}"
    );
    assert!(
        request.contains("Do not rewrite or paraphrase away unique lines"),
        "{request}"
    );
    assert!(
        request.contains("preserve them by wrapping each side in a helper closure, local block, or helper variable"),
        "{request}"
    );
    assert!(
        request.contains("for Rust code, return rustfmt-clean file contents"),
        "{request}"
    );
    assert!(
        request.contains(
            "Do not change function names or call expressions inside required test blocks"
        ),
        "{request}"
    );
    assert!(
        request.contains("\"preservation_requirements\""),
        "{request}"
    );
    assert!(request.contains("\"line\": \"main\""), "{request}");
    assert!(request.contains("\"line\": \"feature\""), "{request}");
}

#[test]
fn merge_fails_when_ai_response_omits_resolve_conflicts_tool_call() {
    let repo = init_conflict_repo();
    write_merge_ai_repo_config(repo.path(), "http://127.0.0.1:9", "", "");
    let (_server, output) = {
        let server = MockAiServer::start(vec![ai_text_response(
            &serde_json::json!({
                "confidence": 0.95,
                "summary": "resolved conflict",
                "files": [{
                    "path": "conflict.txt",
                    "explanation": "apply merged content",
                    "resolved_content": "main\nfeature\n"
                }]
            })
            .to_string(),
        )]);
        write_merge_ai_repo_config(repo.path(), server.url(), "", "");
        let output = run_agent_with_env(
            repo.path(),
            &["--json", "--yes", "merge", "feature"],
            &[("GIT_RAFT_API_KEY", "test-key")],
        );
        (server, output)
    };
    assert!(
        !output.status.success(),
        "plain text merge response should be rejected: {:?}",
        output
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("resolve_conflicts tool call") || stdout.contains("tool call"));
}
