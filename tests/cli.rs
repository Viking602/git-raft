use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
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

fn user_config_file(home: &Path) -> PathBuf {
    home.join(".config/git-raft/config.toml")
}

fn user_commit_examples_file(home: &Path) -> PathBuf {
    home.join(".config/git-raft/commit_examples.md")
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

struct MockAiServer {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    handle: Option<thread::JoinHandle<()>>,
}

enum MockAiResponse {
    Json(Value),
    Sse(Vec<String>),
}

impl From<Value> for MockAiResponse {
    fn from(value: Value) -> Self {
        Self::Json(value)
    }
}

impl MockAiServer {
    fn start<T: Into<MockAiResponse>>(responses: Vec<T>) -> Self {
        let responses = responses.into_iter().map(Into::into).collect::<Vec<_>>();
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock ai server");
        listener
            .set_nonblocking(true)
            .expect("set nonblocking listener");
        let url = format!("http://{}", listener.local_addr().expect("local addr"));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_bg = Arc::clone(&requests);
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut sent = 0usize;
            while started.elapsed() < Duration::from_secs(5) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).expect("set blocking stream");
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .expect("set read timeout");
                        let body = read_http_json_body(&mut stream);
                        if let Some(body) = body {
                            requests_bg.lock().expect("requests lock").push(body);
                        }
                        let response = responses.get(sent).unwrap_or_else(|| {
                            panic!("missing mock response for request {sent}");
                        });
                        sent += 1;
                        write_mock_ai_response(&mut stream, response);
                        if sent >= responses.len() {
                            break;
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(25));
                    }
                    Err(err) => panic!("mock ai accept failed: {err}"),
                }
            }
        });
        Self {
            url,
            requests,
            handle: Some(handle),
        }
    }

    fn url(&self) -> &str {
        &self.url
    }

    fn requests(&self) -> Vec<Value> {
        self.requests.lock().expect("requests lock").clone()
    }
}

impl Drop for MockAiServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().expect("join mock ai server");
        }
    }
}

fn read_http_json_body(stream: &mut std::net::TcpStream) -> Option<Value> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).expect("read request");
        if read == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(pos) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break pos + 4;
        }
    };

    let headers = String::from_utf8_lossy(&buffer[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    while buffer.len() < header_end + content_length {
        let read = stream.read(&mut chunk).expect("read request body");
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }

    if content_length == 0 {
        return None;
    }

    let body = &buffer[header_end..header_end + content_length];
    Some(serde_json::from_slice(body).expect("json request body"))
}

fn write_http_json_response(stream: &mut std::net::TcpStream, body: &Value) {
    let body = serde_json::to_vec(body).expect("response body");
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("write response headers");
    stream.write_all(&body).expect("write response body");
    stream.flush().expect("flush response");
}

fn write_mock_ai_response(stream: &mut std::net::TcpStream, response: &MockAiResponse) {
    match response {
        MockAiResponse::Json(body) => write_http_json_response(stream, body),
        MockAiResponse::Sse(chunks) => write_http_sse_response(stream, chunks),
    }
}

fn write_http_sse_response(stream: &mut std::net::TcpStream, chunks: &[String]) {
    let mut body = Vec::new();
    for chunk in chunks {
        body.extend_from_slice(b"data: ");
        body.extend_from_slice(chunk.as_bytes());
        body.extend_from_slice(b"\n\n");
    }
    body.extend_from_slice(b"data: [DONE]\n\n");
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .expect("write sse headers");
    stream.write_all(&body).expect("write sse body");
    stream.flush().expect("flush sse response");
}

fn ai_text_response(text: &str) -> Value {
    serde_json::json!({
        "choices": [
            {
                "message": {
                    "content": text
                }
            }
        ]
    })
}

fn ai_patch_response(path: &str, content: &str) -> Value {
    ai_text_response(
        &serde_json::json!({
            "confidence": 0.95,
            "summary": "resolved conflict",
            "files": [
                {
                    "path": path,
                    "explanation": "apply merged content",
                    "resolved_content": content
                }
            ]
        })
        .to_string(),
    )
}

fn ai_commit_plan_response(groups: Value, confidence: f32) -> Value {
    ai_text_response(
        &serde_json::json!({
            "groups": groups,
            "confidence": confidence,
            "warnings": [],
            "auto_executable": confidence >= 0.8
        })
        .to_string(),
    )
}

fn ai_commit_plan_stream_response(chunks: &[&str]) -> MockAiResponse {
    MockAiResponse::Sse(
        chunks
            .iter()
            .map(|chunk| {
                serde_json::json!({
                    "choices": [
                        {
                            "delta": {
                                "content": chunk
                            }
                        }
                    ]
                })
                .to_string()
            })
            .collect(),
    )
}

fn commit_group(scope: Option<&str>, files: &[&str], message: &str, rationale: &str) -> Value {
    serde_json::json!({
        "scope": scope,
        "files": files,
        "commit_message": message,
        "rationale": rationale,
    })
}

fn write_commit_ai_repo_config(repo: &Path, base_url: &str, extra_commit: &str, extra_hooks: &str) {
    let commit_use_gitmoji = if extra_commit.contains("use_gitmoji") {
        String::new()
    } else {
        "use_gitmoji = false\n".to_string()
    };
    let commit_language = if extra_commit.contains("language") {
        String::new()
    } else {
        "language = \"en\"\n".to_string()
    };
    let commit_include_body = if extra_commit.contains("include_body") {
        String::new()
    } else {
        "include_body = true\n".to_string()
    };
    let commit_include_footer = if extra_commit.contains("include_footer") {
        String::new()
    } else {
        "include_footer = false\n".to_string()
    };
    let commit_ignore_paths = if extra_commit.contains("ignore_paths") {
        String::new()
    } else {
        "ignore_paths = []\n".to_string()
    };
    let max_group_count = if extra_hooks.contains("max_group_count") {
        String::new()
    } else {
        "max_group_count = 10\n".to_string()
    };
    fs::create_dir_all(config_file(repo).parent().expect("repo config parent"))
        .expect("mkdir repo config");
    fs::write(
        config_file(repo),
        format!(
            r#"[provider]
base_url = "{base_url}"
model = "gpt-4.1-mini"
api_key_env = "GIT_RAFT_API_KEY"

[commit]
format = "conventional"
{commit_use_gitmoji}{commit_language}{commit_include_body}{commit_include_footer}examples_file = ".config/git-raft/commit_examples.md"
{commit_ignore_paths}{extra_commit}

[hooks.rules]
validate_message_format = true
scope_required = false
empty_group = true
{max_group_count}{extra_hooks}
[runs]
dir = ".git/git-raft/runs"
"#,
        ),
    )
    .expect("write commit ai config");
}

fn first_ai_user_request(server: &MockAiServer) -> Value {
    let requests = server.requests();
    let content = requests[0]["messages"][1]["content"]
        .as_str()
        .expect("user content");
    serde_json::from_str(content).expect("parse ai user request")
}

fn run_commit_with_mock_ai<T: Into<MockAiResponse>>(
    repo: &Path,
    responses: Vec<T>,
    extra_commit: &str,
    extra_hooks: &str,
    args: &[&str],
) -> (MockAiServer, std::process::Output) {
    let server = MockAiServer::start(responses);
    write_commit_ai_repo_config(repo, server.url(), extra_commit, extra_hooks);
    let output = run_agent_with_env(repo, args, &[("GIT_RAFT_API_KEY", "test-key")]);
    (server, output)
}

fn write_ai_repo_config(repo: &Path, base_url: &str, extra_hooks: &str) {
    fs::create_dir_all(config_file(repo).parent().expect("repo config parent"))
        .expect("mkdir repo config");
    fs::write(
        config_file(repo),
        format!(
            r#"[provider]
base_url = "{base_url}"
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
{extra_hooks}
[runs]
dir = ".git/git-raft/runs"
"#,
        ),
    )
    .expect("write ai config");
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

    let (server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([
                commit_group(
                    Some("auth"),
                    &["src/auth/mod.rs"],
                    "feat(auth): split auth and docs",
                    "group auth file",
                ),
                commit_group(
                    Some("docs"),
                    &["docs/guide.md"],
                    "docs(docs): split auth and docs",
                    "group docs file",
                )
            ]),
            0.91,
        )],
        "",
        "",
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
    let request = first_ai_user_request(&server);
    let changed_files = request["user_payload"]["changed_files"]
        .as_array()
        .expect("changed files")
        .iter()
        .map(|file| file.as_str().expect("file").to_string())
        .collect::<Vec<_>>();
    assert!(changed_files.contains(&"src/auth/mod.rs".to_string()));
    assert!(changed_files.contains(&"docs/guide.md".to_string()));
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

    let (_server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([
                commit_group(
                    Some("auth"),
                    &["src/auth/mod.rs"],
                    "feat(auth): update auth changes",
                    "auth group",
                ),
                commit_group(
                    Some("docs"),
                    &["docs/guide.md"],
                    "docs(docs): update docs changes",
                    "docs group",
                )
            ]),
            0.91,
        )],
        "",
        "max_group_count = 1",
        &["--json", "commit", "--plan"],
    );
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

    let (_server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([commit_group(
                Some("auth"),
                &["src/auth/mod.rs"],
                "feat(auth): add auth login",
                "group auth file",
            )]),
            0.92,
        )],
        "",
        "",
        &["commit", "--intent", "add auth login"],
    );
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

#[test]
fn commit_executes_even_when_ai_plan_has_low_confidence() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");

    let (_server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([commit_group(
                Some("auth"),
                &["src/auth/mod.rs"],
                "feat(auth): add auth login",
                "group auth file",
            )]),
            0.12,
        )],
        "",
        "",
        &["commit", "--intent", "add auth login"],
    );
    assert!(
        output.status.success(),
        "commit should succeed even with low confidence: {:?}",
        output
    );

    let log = StdCommand::new("git")
        .args(["log", "--oneline", "-1"])
        .current_dir(repo.path())
        .output()
        .expect("git log");
    let subject = String::from_utf8(log.stdout).expect("utf8");
    assert!(subject.contains("feat(auth): add auth login"));
}

#[test]
fn commit_plan_requests_ai_when_provider_is_configured() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    let server = MockAiServer::start(vec![ai_commit_plan_response(
        serde_json::json!([commit_group(
            Some("auth"),
            &["src/auth/mod.rs"],
            "feat(auth): add auth login",
            "grouped auth files",
        )]),
        0.92,
    )]);
    write_commit_ai_repo_config(repo.path(), server.url(), "", "");

    let output = run_agent_with_env(
        repo.path(),
        &["--json", "commit", "--plan"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);

    let events = parse_events(&output.stdout);
    assert!(
        events
            .iter()
            .any(|event| event["event_type"] == "ai_request_started"),
        "expected ai_request_started in commit output: {:?}",
        events
    );
    let request = first_ai_user_request(&server);
    assert_eq!(request["task"], "plan_commit");
}

#[test]
fn commit_plan_streams_ai_deltas() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    let chunks = [
        "{\"groups\":[{\"scope\":\"auth\",\"files\":[\"src/auth/mod.rs\"],",
        "\"commit_message\":\"feat(auth): add auth login\",\"rationale\":\"group auth file\"}],",
        "\"confidence\":0.92,\"warnings\":[],\"auto_executable\":true}",
    ];
    let (_server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_stream_response(&chunks)],
        "",
        "",
        &["--json", "commit", "--plan"],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let delta_count = events
        .iter()
        .filter(|event| event["event_type"] == "ai_response_delta")
        .count();
    assert!(
        delta_count >= 2,
        "expected streamed delta events: {:?}",
        events
    );
    let result = find_tool_result(&events, "commit_plan");
    assert_eq!(
        result["data"]["plan"]["groups"][0]["commit_message"],
        "feat(auth): add auth login"
    );
}

#[test]
fn commit_plan_fails_when_ai_response_is_invalid() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    let server = MockAiServer::start(vec![ai_text_response("not valid commit plan json")]);
    write_commit_ai_repo_config(repo.path(), server.url(), "", "");

    let output = run_agent_with_env(
        repo.path(),
        &["--json", "commit", "--plan"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        !output.status.success(),
        "commit plan unexpectedly succeeded: {:?}",
        output
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8");
    assert!(stdout.contains("\"event_type\":\"ai_response_invalid\""));
}

#[test]
fn commit_plan_groups_root_markdown_files_as_docs() {
    let repo = init_repo();
    fs::write(repo.path().join("README.md"), "hello\nupdated\n").expect("update readme");
    fs::write(repo.path().join("notes.md"), "# notes\n").expect("write notes");

    let (server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([commit_group(
                Some("docs"),
                &["README.md", "notes.md"],
                "docs(docs): document repository notes",
                "group root docs",
            )]),
            0.92,
        )],
        "",
        "",
        &[
            "--json",
            "commit",
            "--plan",
            "--intent",
            "document repository notes",
        ],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "commit_plan");
    let groups = result["data"]["plan"]["groups"]
        .as_array()
        .expect("groups array");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["scope"], "docs");
    assert_eq!(groups[0]["files"].as_array().expect("files").len(), 2);
    let request = first_ai_user_request(&server);
    let changed_files = request["user_payload"]["changed_files"]
        .as_array()
        .expect("changed files")
        .iter()
        .map(|file| file.as_str().expect("file").to_string())
        .collect::<Vec<_>>();
    assert!(changed_files.contains(&"README.md".to_string()));
    assert!(changed_files.contains(&"notes.md".to_string()));
}

#[test]
fn commit_plan_merges_single_code_scope_with_root_companion_files() {
    let repo = init_repo();
    fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write cargo toml");
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    run_git(repo.path(), ["add", "."]);
    run_git(repo.path(), ["commit", "-m", "feat(auth): add auth module"]);

    fs::write(
        repo.path().join("src/auth/mod.rs"),
        "pub fn login() { println!(\"ok\"); }\n",
    )
    .expect("update auth");
    fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.1\"\nedition = \"2024\"\n",
    )
    .expect("update cargo toml");
    fs::write(repo.path().join("README.md"), "hello\nauth update\n").expect("update readme");

    let (_server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([commit_group(
                Some("auth"),
                &["Cargo.toml", "README.md", "src/auth/mod.rs"],
                "refactor(auth): refactor auth setup",
                "group auth and companion files",
            )]),
            0.92,
        )],
        "",
        "",
        &[
            "--json",
            "commit",
            "--plan",
            "--intent",
            "refactor auth setup",
        ],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "commit_plan");
    let groups = result["data"]["plan"]["groups"]
        .as_array()
        .expect("groups array");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["scope"], "auth");
    let files = groups[0]["files"]
        .as_array()
        .expect("files")
        .iter()
        .map(|file| file.as_str().expect("file").to_string())
        .collect::<Vec<_>>();
    assert!(files.contains(&"src/auth/mod.rs".to_string()));
    assert!(files.contains(&"Cargo.toml".to_string()));
    assert!(files.contains(&"README.md".to_string()));
}

#[test]
fn commit_plan_ignores_default_agent_tool_dirs() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join(".codex")).expect("mkdir .codex");
    fs::create_dir_all(repo.path().join(".cursor")).expect("mkdir .cursor");
    fs::create_dir_all(repo.path().join("docs")).expect("mkdir docs");
    fs::write(repo.path().join(".codex/config.toml"), "model = \"test\"\n")
        .expect("write .codex config");
    fs::write(repo.path().join(".cursor/rules.md"), "# rules\n").expect("write .cursor rules");
    fs::write(repo.path().join("docs/guide.md"), "# guide\n").expect("write docs guide");

    let (server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([commit_group(
                Some("docs"),
                &["docs/guide.md"],
                "docs(docs): update docs guide",
                "group docs file",
            )]),
            0.92,
        )],
        "",
        "",
        &["--json", "commit", "--plan"],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "commit_plan");
    let groups = result["data"]["plan"]["groups"]
        .as_array()
        .expect("groups array");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["scope"], "docs");
    let files = groups[0]["files"]
        .as_array()
        .expect("files")
        .iter()
        .map(|file| file.as_str().expect("file").to_string())
        .collect::<Vec<_>>();
    assert_eq!(files, vec!["docs/guide.md".to_string()]);
    let request = first_ai_user_request(&server);
    let changed_files = request["user_payload"]["changed_files"]
        .as_array()
        .expect("changed files")
        .iter()
        .map(|file| file.as_str().expect("file").to_string())
        .collect::<Vec<_>>();
    assert_eq!(changed_files, vec!["docs/guide.md".to_string()]);
}

#[test]
fn commit_plan_honors_custom_ignore_paths() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::create_dir_all(repo.path().join("docs/generated")).expect("mkdir docs generated");
    fs::create_dir_all(repo.path().join(".agents")).expect("mkdir .agents");
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
ignore_paths = ["docs/generated", ".agents", "local.env"]

[runs]
dir = ".git/git-raft/runs"
"#,
    )
    .expect("write config");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    fs::write(
        repo.path().join("docs/generated/report.md"),
        "# generated\n",
    )
    .expect("write generated doc");
    fs::write(repo.path().join(".agents/state.json"), "{}\n").expect("write agent state");
    fs::write(repo.path().join("local.env"), "TOKEN=test\n").expect("write env");

    let (server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([commit_group(
                Some("auth"),
                &["src/auth/mod.rs"],
                "feat(auth): update auth changes",
                "group auth file",
            )]),
            0.92,
        )],
        r#"ignore_paths = ["docs/generated", ".agents", "local.env"]"#,
        "",
        &["--json", "commit", "--plan"],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "commit_plan");
    let groups = result["data"]["plan"]["groups"]
        .as_array()
        .expect("groups array");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["scope"], "auth");
    let files = groups[0]["files"]
        .as_array()
        .expect("files")
        .iter()
        .map(|file| file.as_str().expect("file").to_string())
        .collect::<Vec<_>>();
    assert_eq!(files, vec!["src/auth/mod.rs".to_string()]);
    let request = first_ai_user_request(&server);
    let changed_files = request["user_payload"]["changed_files"]
        .as_array()
        .expect("changed files")
        .iter()
        .map(|file| file.as_str().expect("file").to_string())
        .collect::<Vec<_>>();
    assert_eq!(changed_files, vec!["src/auth/mod.rs".to_string()]);
}

#[test]
fn commit_plan_uses_gitmoji_when_flag_enabled() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");

    let (server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([commit_group(
                Some("auth"),
                &["src/auth/mod.rs"],
                ":sparkles: add auth login",
                "group auth file",
            )]),
            0.92,
        )],
        "use_gitmoji = true",
        "",
        &["--json", "commit", "--plan", "--intent", "add auth login"],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "commit_plan");
    let groups = result["data"]["plan"]["groups"]
        .as_array()
        .expect("groups array");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0]["commit_message"], ":sparkles: add auth login");
    let request = first_ai_user_request(&server);
    assert_eq!(
        request["user_payload"]["format_preferences"]["use_gitmoji"],
        true
    );
}

#[test]
fn commit_plan_uses_chinese_subject_when_configured() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");

    let (server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([commit_group(
                Some("auth"),
                &["src/auth/mod.rs"],
                "feat(auth): 更新 auth 相关改动",
                "group auth file",
            )]),
            0.92,
        )],
        "language = \"zh\"",
        "",
        &["--json", "commit", "--plan"],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "commit_plan");
    let groups = result["data"]["plan"]["groups"]
        .as_array()
        .expect("groups array");
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0]["commit_message"],
        "feat(auth): 更新 auth 相关改动"
    );
    let request = first_ai_user_request(&server);
    assert_eq!(
        request["user_payload"]["format_preferences"]["language"],
        "zh"
    );
}

#[test]
fn commit_language_flag_overrides_config_language() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");

    let (server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([commit_group(
                Some("auth"),
                &["src/auth/mod.rs"],
                "feat(auth): 更新 auth 相关改动",
                "group auth file",
            )]),
            0.92,
        )],
        "language = \"en\"",
        "",
        &["--json", "commit", "--plan", "--language", "zh"],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "commit_plan");
    let groups = result["data"]["plan"]["groups"]
        .as_array()
        .expect("groups array");
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0]["commit_message"],
        "feat(auth): 更新 auth 相关改动"
    );
    let request = first_ai_user_request(&server);
    assert_eq!(
        request["user_payload"]["format_preferences"]["language"],
        "zh"
    );
}

#[test]
fn commit_plan_includes_body_by_default() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");

    let (server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([commit_group(
                Some("auth"),
                &["src/auth/mod.rs"],
                "feat(auth): update auth changes\n\nAffected files:\n- src/auth/mod.rs",
                "group auth file",
            )]),
            0.92,
        )],
        "",
        "",
        &["--json", "commit", "--plan"],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "commit_plan");
    let groups = result["data"]["plan"]["groups"]
        .as_array()
        .expect("groups array");
    assert_eq!(groups.len(), 1);
    let message = groups[0]["commit_message"]
        .as_str()
        .expect("commit message");
    assert!(message.starts_with("feat(auth): update auth changes"));
    assert!(message.contains("\n\nAffected files:\n- src/auth/mod.rs"));
    assert!(!message.contains("\n\nFiles: "));
    let request = first_ai_user_request(&server);
    assert_eq!(
        request["user_payload"]["format_preferences"]["include_body"],
        true
    );
}

#[test]
fn commit_plan_can_append_footer_when_enabled() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");

    let (server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([commit_group(
                Some("auth"),
                &["src/auth/mod.rs"],
                "feat(auth): update auth changes\n\nAffected files:\n- src/auth/mod.rs\n\nFiles: src/auth/mod.rs",
                "group auth file",
            )]),
            0.92,
        )],
        "include_footer = true",
        "",
        &["--json", "commit", "--plan"],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "commit_plan");
    let groups = result["data"]["plan"]["groups"]
        .as_array()
        .expect("groups array");
    assert_eq!(groups.len(), 1);
    let message = groups[0]["commit_message"]
        .as_str()
        .expect("commit message");
    assert!(message.contains("\n\nFiles: src/auth/mod.rs"));
    let request = first_ai_user_request(&server);
    assert_eq!(
        request["user_payload"]["format_preferences"]["include_footer"],
        true
    );
}

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
