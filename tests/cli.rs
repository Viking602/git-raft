use serde_json::Value;
use std::fs;
use std::process::Command as StdCommand;
use tempfile::TempDir;
mod support;

use support::*;

#[test]
fn status_json_emits_events_and_persists_trace() {
    let repo = init_repo();
    let output = run_agent(repo.path(), &["--json", "status"]);
    assert!(output.status.success(), "status failed: {:?}", output);

    let lines = parse_events(&output.stdout);

    let kinds = lines
        .iter()
        .map(|line| line["event_type"].as_str().expect("event_type"))
        .collect::<Vec<_>>();
    assert!(kinds.contains(&"run_started"));
    assert!(kinds.contains(&"phase_changed"));
    assert!(kinds.contains(&"run_finished"));

    let run_dir = latest_run_dir(repo.path());
    assert!(run_dir.join("events.ndjson").exists());
    assert!(run_dir.join("run.json").exists());
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
fn doctor_reports_git_status() {
    let repo = init_repo();
    let output = run_agent(repo.path(), &["--json", "doctor"]);
    assert!(output.status.success(), "doctor failed: {:?}", output);
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("\"event_type\":\"run_finished\""));
    assert!(stdout.contains("\"git_available\":true"));
}

#[test]
fn runs_lists_previous_runs() {
    let repo = init_repo();
    let first = run_agent(repo.path(), &["status"]);
    assert!(first.status.success(), "status failed");

    let output = run_agent(repo.path(), &["runs"]);
    assert!(output.status.success(), "runs failed");
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("status"));
}

#[test]
fn rollback_restores_saved_backup_ref() {
    let repo = init_repo();
    let merge = run_agent(repo.path(), &["--yes", "--json", "merge", "missing-branch"]);
    assert!(
        !merge.status.success(),
        "merge should fail on missing branch"
    );

    let run_dir = latest_run_dir(repo.path());
    let run_json = fs::read_to_string(run_dir.join("run.json")).expect("run json");
    let run: Value = serde_json::from_str(&run_json).expect("run value");
    let run_id = run["run_id"].as_str().expect("run id");

    fs::write(repo.path().join("README.md"), "changed\n").expect("change readme");
    let rollback = run_agent(repo.path(), &["--yes", "rollback", run_id]);
    assert!(rollback.status.success(), "rollback failed: {:?}", rollback);

    let readme = fs::read_to_string(repo.path().join("README.md")).expect("readme");
    assert_eq!(readme, "hello\n");
}

#[test]
fn status_does_not_auto_generate_repo_config() {
    let repo = init_repo();
    let config = config_file(repo.path());
    let examples = commit_examples_file(repo.path());

    let output = run_agent(repo.path(), &["status"]);
    assert!(output.status.success(), "status failed");
    assert!(!config.exists(), "status should not create repo config");
    assert!(
        !examples.exists(),
        "status should not create commit examples"
    );
}

#[test]
fn init_repo_scope_generates_repo_config_without_overwriting_existing_content() {
    let repo = init_repo();
    let config = config_file(repo.path());

    let first = run_agent(repo.path(), &["init", "--project"]);
    assert!(first.status.success(), "repo init failed");
    let generated = fs::read_to_string(&config).expect("generated config");
    assert!(generated.contains("git-raft"));
    assert!(generated.contains("base_url"));
    assert!(generated.contains("api_key = \"\""));
    assert!(generated.contains("api_key_env"));
    assert!(generated.contains("format = \"conventional\""));
    assert!(generated.contains("use_gitmoji = false"));
    assert!(generated.contains("language = \"en\""));
    assert!(generated.contains("include_body = true"));
    assert!(generated.contains("include_footer = false"));
    assert!(generated.contains("examples_file = \".config/git-raft/commit_examples.md\""));
    assert!(generated.contains("ignore_paths = []"));

    let examples = fs::read_to_string(commit_examples_file(repo.path())).expect("examples file");
    assert!(examples.contains("## conventional"));
    assert!(examples.contains("## gitmoji"));
    assert!(examples.contains("## simple"));

    fs::write(&config, "custom = true\n").expect("custom config");
    let second = run_agent(repo.path(), &["init", "--project"]);
    assert!(second.status.success(), "second repo init failed");
    let preserved = fs::read_to_string(&config).expect("preserved config");
    assert_eq!(preserved, "custom = true\n");
}

#[test]
fn init_defaults_to_user_scope() {
    let repo = init_repo();
    let home = TempDir::new().expect("home");
    let home_str = home.path().display().to_string();

    let output = run_agent_with_env(repo.path(), &["init"], &[("HOME", &home_str)]);
    assert!(output.status.success(), "default init failed: {:?}", output);

    let config = fs::read_to_string(user_config_file(home.path())).expect("user config");
    assert!(config.contains("format = \"conventional\""));
    assert!(config.contains("language = \"en\""));
    assert!(config.contains("include_body = true"));
    assert!(config.contains("include_footer = false"));
    assert!(
        config.contains(&format!(
            "examples_file = \"{}\"",
            user_commit_examples_file(home.path()).display()
        )),
        "unexpected examples_file path: {config}"
    );

    let examples =
        fs::read_to_string(user_commit_examples_file(home.path())).expect("user commit examples");
    assert!(examples.contains("## conventional"));
    assert!(
        !config_file(repo.path()).exists(),
        "default init should not create repo config"
    );
}

#[test]
fn init_defaults_to_user_scope_outside_repo() {
    let dir = TempDir::new().expect("dir");
    let home = TempDir::new().expect("home");
    let home_str = home.path().display().to_string();

    let output = run_agent_with_env(dir.path(), &["init"], &[("HOME", &home_str)]);
    assert!(
        output.status.success(),
        "default init outside repo failed: {:?}",
        output
    );
    assert!(user_config_file(home.path()).exists());
    assert!(user_commit_examples_file(home.path()).exists());
}

#[test]
fn doctor_reads_model_and_commit_format_from_repo_config() {
    let repo = init_repo();
    let config = config_file(repo.path());
    let examples = commit_examples_file(repo.path());
    fs::create_dir_all(config.parent().expect("config parent")).expect("create config dir");
    fs::write(
        &config,
        r#"[provider]
base_url = "https://example.test/v1"
model = "gpt-5.4"
api_key_env = "GIT_RAFT_API_KEY"

[commit]
format = "gitmoji"
examples_file = ".config/git-raft/commit_examples.md"

[runs]
dir = ".git/git-raft/runs"
"#,
    )
    .expect("write config");
    fs::write(&examples, "# custom examples\n").expect("write examples");

    let output = run_agent(repo.path(), &["--json", "doctor"]);
    assert!(output.status.success(), "doctor failed: {:?}", output);
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("\"provider_model\":\"gpt-5.4\""));
    assert!(stdout.contains("\"commit_format\":\"gitmoji\""));
    assert!(stdout.contains("\"commit_examples_file\":\".config/git-raft/commit_examples.md\""));
}

#[test]
fn doctor_accepts_inline_provider_api_key() {
    let repo = init_repo();
    let config = config_file(repo.path());
    fs::create_dir_all(config.parent().expect("config parent")).expect("create config dir");
    fs::write(
        &config,
        r#"[provider]
base_url = "https://example.test/v1"
model = "gpt-5.4"
api_key = "inline-test-key"
api_key_env = "MISSING_KEY"

[commit]
format = "conventional"
examples_file = ".config/git-raft/commit_examples.md"

[runs]
dir = ".git/git-raft/runs"
"#,
    )
    .expect("write config");

    let output = run_agent(repo.path(), &["--json", "doctor"]);
    assert!(output.status.success(), "doctor failed: {:?}", output);
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("\"provider_configured\":true"));
}

#[test]
fn config_show_merges_user_and_repo_sources() {
    let repo = init_repo();
    let home = TempDir::new().expect("home");
    let user_config = home.path().join(".config/git-raft/config.toml");
    fs::create_dir_all(user_config.parent().expect("user config parent"))
        .expect("mkdir user config");
    fs::write(
        &user_config,
        r#"[provider]
base_url = "https://user.test/v1"
model = "user-model"
api_key_env = "USER_KEY"

[commit]
format = "simple"
examples_file = ".config/git-raft/commit_examples.md"
"#,
    )
    .expect("write user config");

    fs::create_dir_all(
        config_file(repo.path())
            .parent()
            .expect("repo config parent"),
    )
    .expect("mkdir repo config");
    fs::write(
        config_file(repo.path()),
        r#"[provider]
base_url = "https://repo.test/v1"
model = "repo-model"
api_key_env = "REPO_KEY"

[commit]
format = "gitmoji"
examples_file = ".config/git-raft/commit_examples.md"

[runs]
dir = ".git/git-raft/runs"
"#,
    )
    .expect("write repo config");

    let home_str = home.path().display().to_string();
    let output = run_agent_with_env(
        repo.path(),
        &["--json", "config", "show", "--scope", "resolved"],
        &[("HOME", &home_str)],
    );
    assert!(output.status.success(), "config show failed: {:?}", output);

    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "config_show");
    assert_eq!(result["data"]["scope"], "resolved");
    assert_eq!(result["data"]["config"]["provider"]["model"], "repo-model");
    assert_eq!(result["data"]["config"]["commit"]["format"], "gitmoji");
    assert_eq!(result["data"]["sources"]["provider.model"], "repo");
    assert_eq!(result["data"]["sources"]["runs.dir"], "repo");
}

#[test]
fn config_get_accepts_kebab_case_and_reports_source() {
    let repo = init_repo();
    fs::create_dir_all(
        config_file(repo.path())
            .parent()
            .expect("repo config parent"),
    )
    .expect("mkdir repo config");
    fs::write(
        config_file(repo.path()),
        r#"[provider]
base_url = "https://repo.test/v1"
model = "gpt-5.4"
api_key_env = "GIT_RAFT_API_KEY"

[commit]
format = "angular"
examples_file = ".config/git-raft/commit_examples.md"
"#,
    )
    .expect("write repo config");

    let output = run_agent(repo.path(), &["--json", "config", "get", "commit-format"]);
    assert!(output.status.success(), "config get failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "config_get");
    assert_eq!(result["data"]["key"], "commit.format");
    assert_eq!(result["data"]["value"], "angular");
    assert_eq!(result["data"]["source"], "repo");
}

#[test]
fn config_set_user_scope_creates_user_config() {
    let repo = init_repo();
    let home = TempDir::new().expect("home");
    let home_str = home.path().display().to_string();

    let output = run_agent_with_env(
        repo.path(),
        &[
            "config",
            "set",
            "provider-model",
            "gpt-5.4",
            "--scope",
            "user",
        ],
        &[("HOME", &home_str)],
    );
    assert!(output.status.success(), "config set failed: {:?}", output);

    let user_config = home.path().join(".config/git-raft/config.toml");
    let content = fs::read_to_string(user_config).expect("user config content");
    assert!(content.contains("model = \"gpt-5.4\""));
}

#[test]
fn scopes_generate_persists_and_lists_scope_candidates() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::create_dir_all(repo.path().join("docs")).expect("mkdir docs");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    fs::write(repo.path().join("src/cli.rs"), "pub fn run() {}\n").expect("write cli");
    fs::write(repo.path().join("docs/guide.md"), "# guide\n").expect("write docs");
    run_git(repo.path(), ["add", "."]);
    run_git(
        repo.path(),
        ["commit", "-m", "feat(auth): add login module"],
    );
    fs::write(
        repo.path().join("src/cli.rs"),
        "pub fn run() { println!(\"hi\"); }\n",
    )
    .expect("update cli");
    run_git(repo.path(), ["add", "src/cli.rs"]);
    run_git(
        repo.path(),
        ["commit", "-m", "fix(cli): adjust command output"],
    );

    let generate = run_agent(repo.path(), &["--json", "scopes", "generate"]);
    assert!(
        generate.status.success(),
        "scopes generate failed: {:?}",
        generate
    );
    let list = run_agent(repo.path(), &["--json", "scopes", "list"]);
    assert!(list.status.success(), "scopes list failed: {:?}", list);

    let events = parse_events(&list.stdout);
    let result = find_tool_result(&events, "scopes_list");
    let scopes = result["data"]["scopes"].as_array().expect("scopes array");
    assert!(scopes.iter().any(|scope| scope["name"] == "auth"));
    assert!(scopes.iter().any(|scope| scope["name"] == "cli"));

    let config = fs::read_to_string(config_file(repo.path())).expect("repo config");
    assert!(config.contains("[[commit.scopes]]"));
}

#[test]
fn external_before_command_hook_receives_camel_case_payload() {
    let repo = init_repo();
    let hook_dir = repo.path().join(".config/git-raft");
    fs::create_dir_all(&hook_dir).expect("hook dir");
    let script = hook_dir.join("capture.sh");
    fs::write(&script, "cat > \"$1\"\n").expect("script");
    let payload_path = ".config/git-raft/hook-payload.json";
    fs::write(
        config_file(repo.path()),
        format!(
            r#"[provider]
base_url = ""
model = "gpt-4.1-mini"
api_key_env = "GIT_RAFT_API_KEY"

[commit]
format = "conventional"
examples_file = ".config/git-raft/commit_examples.md"

[hooks.rules]
validate_message_format = true
scope_required = false
empty_group = true
max_group_count = 10

[[hooks.external]]
event = "beforeCommand"
program = "sh"
args = [".config/git-raft/capture.sh", "{payload_path}"]

[runs]
dir = ".git/git-raft/runs"
"#,
        ),
    )
    .expect("write config");

    let output = run_agent(repo.path(), &["status"]);
    assert!(output.status.success(), "status failed: {:?}", output);
    let payload = fs::read_to_string(repo.path().join(payload_path)).expect("payload");
    assert!(payload.contains("\"event\":\"beforeCommand\""));
    assert!(payload.contains("\"repoRoot\""));
    assert!(payload.contains("\"timestampMs\""));
    assert!(payload.contains("\"gitSnapshot\""));
}

#[path = "cli_cases/commit.rs"]
mod commit_cases;

#[test]
fn ask_persists_structured_ai_request_and_emits_ai_events() {
    let repo = init_repo();
    let server = MockAiServer::start(vec![ai_text_response("answer from mock ai")]);
    write_ai_repo_config(repo.path(), server.url(), "");

    let output = run_agent_with_env(
        repo.path(),
        &["--json", "ask", "summarize", "this", "repo"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(output.status.success(), "ask failed: {:?}", output);
    assert_eq!(server.requests().len(), 1);

    let events = parse_events(&output.stdout);
    assert!(
        events
            .iter()
            .any(|event| event["event_type"] == "ai_request_started"),
        "missing ai_request_started event: {:?}",
        events
    );
    assert!(
        events
            .iter()
            .any(|event| event["event_type"] == "ai_response_ready"),
        "missing ai_response_ready event: {:?}",
        events
    );

    let run_dir = latest_run_dir(repo.path());
    let request: Value = serde_json::from_str(
        &fs::read_to_string(run_dir.join("ai-request.json")).expect("ai-request.json"),
    )
    .expect("parse ai request");

    assert_eq!(request["task"], "ask");
    assert_eq!(
        request["request"]["user_payload"]["prompt"],
        "summarize this repo"
    );
    assert_eq!(request["request"]["repo_context"]["branch"], "main");
    assert_eq!(
        request["request"]["repo_context"]["commit_format"],
        "conventional"
    );
    assert!(
        request["request"]["repo_context"]["recent_subjects"]
            .as_array()
            .is_some()
    );
    assert!(
        request["request"]["repo_context"]["diff_stats"]
            .as_array()
            .is_some()
    );
}

#[test]
fn ask_uses_inline_provider_api_key_when_env_is_missing() {
    let repo = init_repo();
    let server = MockAiServer::start(vec![ai_text_response("answer from inline key")]);
    fs::create_dir_all(
        config_file(repo.path())
            .parent()
            .expect("repo config parent"),
    )
    .expect("mkdir repo config");
    fs::write(
        config_file(repo.path()),
        format!(
            r#"[provider]
base_url = "{}"
model = "gpt-4.1-mini"
api_key = "inline-test-key"
api_key_env = "MISSING_KEY"

[commit]
format = "conventional"
examples_file = ".config/git-raft/commit_examples.md"

[runs]
dir = ".git/git-raft/runs"
"#,
            server.url()
        ),
    )
    .expect("write config");

    let output = run_agent(repo.path(), &["ask", "summarize", "repo"]);
    assert!(
        output.status.success(),
        "ask failed without env api key: {:?}",
        output
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("answer from inline key"));
}

#[test]
fn before_ai_request_hook_can_block_request() {
    let repo = init_repo();
    let hook_dir = repo.path().join(".config/git-raft");
    fs::create_dir_all(&hook_dir).expect("hook dir");
    let script = hook_dir.join("block-ai.sh");
    fs::write(
        &script,
        "cat > \"$1\"\nprintf '{\"blocked\":true,\"reason\":\"blocked before ai request\"}'\n",
    )
    .expect("write block script");
    let payload_path = ".config/git-raft/before-ai-request.json";
    write_ai_repo_config(
        repo.path(),
        "http://127.0.0.1:9",
        &format!(
            r#"

[[hooks.external]]
event = "beforeAiRequest"
program = "sh"
args = [".config/git-raft/block-ai.sh", "{payload_path}"]
"#,
        ),
    );

    let output = run_agent_with_env(
        repo.path(),
        &["--json", "ask", "blocked", "request"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        !output.status.success(),
        "ask unexpectedly succeeded: {:?}",
        output
    );

    let payload = fs::read_to_string(repo.path().join(payload_path)).expect("hook payload");
    assert!(payload.contains("\"event\":\"beforeAiRequest\""));
    assert!(payload.contains("\"agentTask\":\"ask\""));
    assert!(payload.contains("\"agentRequestSummary\""));

    let run_dir = latest_run_dir(repo.path());
    assert!(
        !run_dir.join("ai-request.json").exists(),
        "blocked request should not persist ai-request.json"
    );
}

#[test]
fn before_patch_apply_hook_blocks_apply_and_keeps_patch_json() {
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

    let hook_dir = repo.path().join(".config/git-raft");
    fs::create_dir_all(&hook_dir).expect("hook dir");
    let script = hook_dir.join("block-patch.sh");
    fs::write(
        &script,
        "cat > \"$1\"\nprintf '{\"blocked\":true,\"reason\":\"blocked before patch apply\"}'\n",
    )
    .expect("write patch blocker");
    let payload_path = ".config/git-raft/before-patch-apply.json";
    let server = MockAiServer::start(vec![ai_patch_response("conflict.txt", "resolved\n")]);
    write_ai_repo_config(
        repo.path(),
        server.url(),
        &format!(
            r#"

[[hooks.external]]
event = "beforePatchApply"
program = "sh"
args = [".config/git-raft/block-patch.sh", "{payload_path}"]
"#,
        ),
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
