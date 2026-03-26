use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use tempfile::TempDir;

fn init_repo() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    run_git(dir.path(), ["init", "--initial-branch=main"]);
    run_git(dir.path(), ["config", "user.name", "Test User"]);
    run_git(dir.path(), ["config", "user.email", "test@example.com"]);
    fs::write(dir.path().join("README.md"), "hello\n").expect("write readme");
    run_git(dir.path(), ["add", "README.md"]);
    run_git(dir.path(), ["commit", "-m", "init"]);
    dir
}

fn run_git<I, S>(cwd: &Path, args: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let status = StdCommand::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .expect("spawn git");
    assert!(status.success(), "git command failed");
}

fn latest_run_dir(repo: &Path) -> PathBuf {
    let root = repo.join(".git/git-raft/runs");
    let mut entries = fs::read_dir(&root)
        .expect("runs dir exists")
        .map(|entry| entry.expect("entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    entries.pop().expect("latest run dir")
}

fn config_file(repo: &Path) -> PathBuf {
    repo.join(".config/git-raft/config.toml")
}

fn commit_examples_file(repo: &Path) -> PathBuf {
    repo.join(".config/git-raft/commit_examples.md")
}

fn run_agent(repo: &Path, args: &[&str]) -> std::process::Output {
    StdCommand::new(env!("CARGO_BIN_EXE_git-raft"))
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git-raft")
}

fn run_agent_with_env(repo: &Path, args: &[&str], envs: &[(&str, &str)]) -> std::process::Output {
    let mut command = StdCommand::new(env!("CARGO_BIN_EXE_git-raft"));
    command.args(args).current_dir(repo);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("run git-raft with env")
}

fn parse_events(output: &[u8]) -> Vec<Value> {
    String::from_utf8(output.to_vec())
        .expect("utf8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("valid ndjson"))
        .collect()
}

fn find_tool_result<'a>(events: &'a [Value], message: &str) -> &'a Value {
    events
        .iter()
        .find(|event| {
            event["event_type"] == "tool_result" && event["message"].as_str() == Some(message)
        })
        .expect("tool result event")
}

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
fn status_auto_generates_repo_config_without_overwriting_existing_content() {
    let repo = init_repo();
    let config = config_file(repo.path());

    let first = run_agent(repo.path(), &["status"]);
    assert!(first.status.success(), "status failed");
    let generated = fs::read_to_string(&config).expect("generated config");
    assert!(generated.contains("git-raft"));
    assert!(generated.contains("base_url"));
    assert!(generated.contains("api_key_env"));
    assert!(generated.contains("format = \"conventional\""));
    assert!(generated.contains("examples_file = \".config/git-raft/commit_examples.md\""));

    let examples = fs::read_to_string(commit_examples_file(repo.path())).expect("examples file");
    assert!(examples.contains("## conventional"));
    assert!(examples.contains("## gitmoji"));
    assert!(examples.contains("## simple"));

    fs::write(&config, "custom = true\n").expect("custom config");
    let second = run_agent(repo.path(), &["status"]);
    assert!(second.status.success(), "second status failed");
    let preserved = fs::read_to_string(&config).expect("preserved config");
    assert_eq!(preserved, "custom = true\n");
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

#[test]
fn commit_plan_combines_staged_and_unstaged_changes() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::create_dir_all(repo.path().join("docs")).expect("mkdir docs");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    fs::write(repo.path().join("docs/guide.md"), "# guide\n").expect("write docs");
    run_git(repo.path(), ["add", "."]);
    run_git(
        repo.path(),
        ["commit", "-m", "feat(auth): add auth skeleton"],
    );

    fs::write(
        repo.path().join("src/auth/mod.rs"),
        "pub fn login() { println!(\"x\"); }\n",
    )
    .expect("update auth");
    fs::write(repo.path().join("docs/guide.md"), "# guide\nupdated\n").expect("update docs");
    run_git(repo.path(), ["add", "src/auth/mod.rs"]);

    let output = run_agent(
        repo.path(),
        &[
            "--json",
            "commit",
            "--plan",
            "--intent",
            "split auth and docs",
        ],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "commit_plan");
    let groups = result["data"]["plan"]["groups"]
        .as_array()
        .expect("groups array");
    assert_eq!(groups.len(), 2);
    let files = result["data"]["plan"]["groups"]
        .as_array()
        .expect("groups array")
        .iter()
        .flat_map(|group| group["files"].as_array().expect("files"))
        .map(|file| file.as_str().expect("file").to_string())
        .collect::<Vec<_>>();
    assert!(files.contains(&"src/auth/mod.rs".to_string()));
    assert!(files.contains(&"docs/guide.md".to_string()));
}

#[test]
fn commit_plan_is_blocked_by_max_group_count_rule() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::create_dir_all(repo.path().join("docs")).expect("mkdir docs");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    fs::write(repo.path().join("docs/guide.md"), "# guide\n").expect("write docs");
    fs::create_dir_all(
        config_file(repo.path())
            .parent()
            .expect("repo config parent"),
    )
    .expect("mkdir repo config");
    fs::write(
        config_file(repo.path()),
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
max_group_count = 1

[runs]
dir = ".git/git-raft/runs"
"#,
    )
    .expect("write config");

    let output = run_agent(repo.path(), &["--json", "commit", "--plan"]);
    assert!(
        !output.status.success(),
        "commit plan unexpectedly succeeded"
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(
        stdout.contains("\"event_type\":\"commandFailed\"") || stdout.contains("\"blocked\":true")
    );
}

#[test]
fn commit_executes_single_group_when_plan_is_confident() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");

    let output = run_agent(repo.path(), &["commit", "--intent", "add auth login"]);
    assert!(output.status.success(), "commit failed: {:?}", output);

    let log = StdCommand::new("git")
        .args(["log", "--oneline", "-1"])
        .current_dir(repo.path())
        .output()
        .expect("git log");
    assert!(log.status.success(), "git log failed");
    let subject = String::from_utf8(log.stdout).expect("utf8");
    assert!(subject.contains("feat(auth): add auth login"));
}
