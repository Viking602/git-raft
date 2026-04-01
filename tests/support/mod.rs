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

pub(crate) fn init_repo() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    run_git(dir.path(), ["init", "--initial-branch=main"]);
    run_git(dir.path(), ["config", "user.name", "Test User"]);
    run_git(dir.path(), ["config", "user.email", "test@example.com"]);
    run_git(dir.path(), ["config", "core.autocrlf", "false"]);
    run_git(dir.path(), ["config", "core.eol", "lf"]);
    fs::write(dir.path().join("README.md"), "hello\n").expect("write readme");
    run_git(dir.path(), ["add", "README.md"]);
    run_git(dir.path(), ["commit", "-m", "init"]);
    dir
}

pub(crate) fn run_git<I, S>(cwd: &Path, args: I)
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

pub(crate) fn latest_run_dir(repo: &Path) -> PathBuf {
    let root = repo.join(".git/git-raft/runs");
    let mut entries = fs::read_dir(&root)
        .expect("runs dir exists")
        .map(|entry| entry.expect("entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    entries.pop().expect("latest run dir")
}

pub(crate) fn config_file(repo: &Path) -> PathBuf {
    repo.join(".config/git-raft/config.toml")
}

pub(crate) fn run_agent(repo: &Path, args: &[&str]) -> std::process::Output {
    let mut command = StdCommand::new(env!("CARGO_BIN_EXE_git-raft"));
    command.args(args).current_dir(repo);
    apply_test_home(&mut command, repo, &[]);
    command.output().expect("run git-raft")
}

pub(crate) fn run_agent_with_env(
    repo: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
) -> std::process::Output {
    let mut command = StdCommand::new(env!("CARGO_BIN_EXE_git-raft"));
    command.args(args).current_dir(repo);
    apply_test_home(&mut command, repo, envs);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("run git-raft with env")
}

#[cfg(unix)]
pub(crate) fn path_with_slow_git_status(repo: &Path, delay_ms: u64) -> String {
    let output = StdCommand::new("sh")
        .args(["-lc", "command -v git"])
        .output()
        .expect("locate git");
    assert!(output.status.success(), "failed to locate git");
    let git_path = String::from_utf8(output.stdout)
        .expect("git path utf8")
        .trim()
        .to_string();

    let bin_dir = repo.join(".test-bin");
    fs::create_dir_all(&bin_dir).expect("create test bin dir");
    let script_path = bin_dir.join("git");
    fs::write(
        &script_path,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"status\" ]; then\n  sleep {}\nfi\nexec \"{}\" \"$@\"\n",
            delay_ms as f64 / 1000.0,
            git_path
        ),
    )
    .expect("write slow git wrapper");
    let metadata = fs::metadata(&script_path).expect("slow git metadata");
    let mut permissions = metadata.permissions();
    use std::os::unix::fs::PermissionsExt;
    permissions.set_mode(0o755);
    fs::set_permissions(&script_path, permissions).expect("chmod slow git wrapper");

    let current_path = std::env::var("PATH").expect("PATH");
    format!("{}:{}", bin_dir.display(), current_path)
}

pub(crate) struct HookCommand {
    pub program: String,
    pub args: Vec<String>,
}

pub(crate) fn write_external_hook(
    repo: &Path,
    script_name: &str,
    payload_path: &str,
    response_json: Option<&str>,
) -> HookCommand {
    let hook_dir = repo.join(".config/git-raft");
    fs::create_dir_all(&hook_dir).expect("hook dir");

    if cfg!(windows) {
        let script_rel = format!(".config/git-raft/{script_name}.ps1");
        let script_path = hook_dir.join(format!("{script_name}.ps1"));
        let mut script = String::from(
            "$payload = [Console]::In.ReadToEnd()\n[System.IO.File]::WriteAllText($args[0], $payload)\n",
        );
        if let Some(response_json) = response_json {
            script.push_str(&format!(
                "Write-Output '{}'\n",
                response_json.replace('\'', "''")
            ));
        }
        fs::write(&script_path, script).expect("write powershell hook");
        HookCommand {
            program: "powershell.exe".to_string(),
            args: vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                script_rel,
                payload_path.to_string(),
            ],
        }
    } else {
        let script_rel = format!(".config/git-raft/{script_name}.sh");
        let script_path = hook_dir.join(format!("{script_name}.sh"));
        let mut script = String::from("#!/bin/sh\ncat > \"$1\"\n");
        if let Some(response_json) = response_json {
            script.push_str(&format!("printf '%s' '{}'\n", response_json));
        }
        fs::write(&script_path, script).expect("write shell hook");
        HookCommand {
            program: "sh".to_string(),
            args: vec![script_rel, payload_path.to_string()],
        }
    }
}

pub(crate) fn external_hook_toml(event: &str, hook: &HookCommand) -> String {
    let args = hook
        .args
        .iter()
        .map(|arg| format!("{arg:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"

[[hooks.external]]
event = "{event}"
program = {program:?}
args = [{args}]
"#,
        program = hook.program
    )
}

pub(crate) fn merge_verification_toml(repair_attempts: usize, commands: &[HookCommand]) -> String {
    let mut toml = format!(
        r#"

[merge]
repair_attempts = {repair_attempts}
"#
    );
    for command in commands {
        let args = command
            .args
            .iter()
            .map(|arg| format!("{arg:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        toml.push_str(&format!(
            r#"

[[merge.verification]]
program = {program:?}
args = [{args}]
"#,
            program = command.program
        ));
    }
    toml
}

pub(crate) fn write_repo_command_script(repo: &Path, script_name: &str, body: &str) -> HookCommand {
    let command_dir = repo.join(".config/git-raft");
    fs::create_dir_all(&command_dir).expect("command dir");

    if cfg!(windows) {
        let script_rel = format!(".config/git-raft/{script_name}.ps1");
        let script_path = command_dir.join(format!("{script_name}.ps1"));
        fs::write(&script_path, body).expect("write powershell command");
        HookCommand {
            program: "powershell.exe".to_string(),
            args: vec![
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-File".to_string(),
                script_rel,
            ],
        }
    } else {
        let script_rel = format!(".config/git-raft/{script_name}.sh");
        let script_path = command_dir.join(format!("{script_name}.sh"));
        fs::write(&script_path, body).expect("write shell command");
        HookCommand {
            program: "sh".to_string(),
            args: vec![script_rel],
        }
    }
}

fn apply_test_home(command: &mut StdCommand, repo: &Path, envs: &[(&str, &str)]) {
    let home = envs
        .iter()
        .find_map(|(key, value)| match *key {
            "HOME" | "USERPROFILE" => Some((*value).to_string()),
            _ => None,
        })
        .unwrap_or_else(|| repo.with_extension("home").display().to_string());
    fs::create_dir_all(&home).expect("create test home");
    command.env("HOME", &home);
    command.env("USERPROFILE", &home);
}

pub(crate) struct MockAiServer {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    handle: Option<thread::JoinHandle<()>>,
}

pub(crate) enum MockAiResponse {
    Json(Value),
}

impl From<Value> for MockAiResponse {
    fn from(value: Value) -> Self {
        Self::Json(value)
    }
}

impl MockAiServer {
    pub(crate) fn start<T: Into<MockAiResponse>>(responses: Vec<T>) -> Self {
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
            while started.elapsed() < Duration::from_secs(15) {
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

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    pub(crate) fn requests(&self) -> Vec<Value> {
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
    }
}

pub(crate) fn ai_text_response(text: &str) -> Value {
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

pub(crate) fn ai_patch_response(path: &str, content: &str) -> Value {
    serde_json::json!({
        "choices": [
            {
                "message": {
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_resolve_conflicts",
                            "type": "function",
                            "function": {
                                "name": "resolve_conflicts",
                                "arguments": serde_json::json!({
                                    "confidence": 0.95,
                                    "summary": "resolved conflict",
                                    "files": [
                                        {
                                            "path": path,
                                            "explanation": "apply merged content",
                                            "resolved_content": content
                                        }
                                    ]
                                }).to_string()
                            }
                        }
                    ]
                }
            }
        ]
    })
}

pub(crate) fn ai_commit_plan_response(groups: Value, confidence: f32) -> Value {
    ai_commit_plan_tool_response(groups, confidence)
}

pub(crate) fn ai_commit_plan_tool_response(groups: Value, confidence: f32) -> Value {
    ai_commit_plan_tool_response_with_decision(groups, confidence, None, None)
}

pub(crate) fn ai_commit_plan_response_with_decision(
    groups: Value,
    confidence: f32,
    grouping_decision: Option<&str>,
    grouping_confidence: Option<f32>,
) -> Value {
    ai_commit_plan_tool_response_with_decision(
        groups,
        confidence,
        grouping_decision,
        grouping_confidence,
    )
}

pub(crate) fn ai_commit_plan_tool_response_with_decision(
    groups: Value,
    confidence: f32,
    grouping_decision: Option<&str>,
    grouping_confidence: Option<f32>,
) -> Value {
    let groups_array = groups.as_array().cloned().expect("groups array");
    let single_group = build_single_group(&groups_array);
    let decision = grouping_decision.unwrap_or(if groups_array.len() > 1 {
        "split"
    } else {
        "single"
    });
    serde_json::json!({
        "choices": [
            {
                "message": {
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call_plan_commit",
                            "type": "function",
                            "function": {
                                "name": "plan_commit",
                                "arguments": serde_json::json!({
                                    "grouping_decision": decision,
                                    "grouping_confidence": grouping_confidence.unwrap_or(confidence),
                                    "single_group": single_group,
                                    "groups": groups_array,
                                    "confidence": confidence,
                                    "warnings": [],
                                    "auto_executable": true
                                }).to_string()
                            }
                        }
                    ]
                }
            }
        ]
    })
}

pub(crate) fn commit_group(
    scope: Option<&str>,
    files: &[&str],
    message: &str,
    rationale: &str,
) -> Value {
    serde_json::json!({
        "scope": scope,
        "files": files,
        "commit_message": message,
        "rationale": rationale,
    })
}

pub(crate) fn build_single_group(groups: &[Value]) -> Value {
    if groups.len() == 1 {
        return groups[0].clone();
    }
    let mut files = groups
        .iter()
        .flat_map(|group| {
            group["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|file| file.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    files.sort();
    files.dedup();
    let commit_message = groups
        .first()
        .and_then(|group| group["commit_message"].as_str())
        .unwrap_or("feat: update changes");
    serde_json::json!({
        "scope": null,
        "files": files,
        "commit_message": commit_message,
        "rationale": "single commit fallback"
    })
}

pub(crate) fn write_commit_ai_repo_config(
    repo: &Path,
    base_url: &str,
    extra_commit: &str,
    extra_hooks: &str,
) {
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

pub(crate) fn first_ai_user_request(server: &MockAiServer) -> Value {
    nth_ai_user_request(server, 0)
}

pub(crate) fn nth_ai_user_request(server: &MockAiServer, index: usize) -> Value {
    let requests = server.requests();
    let content = requests[index]["messages"][1]["content"]
        .as_str()
        .expect("user content");
    serde_json::from_str(content).expect("parse ai user request")
}

pub(crate) fn first_ai_provider_request(server: &MockAiServer) -> Value {
    server.requests()[0].clone()
}

pub(crate) fn run_commit_with_mock_ai<T: Into<MockAiResponse>>(
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

pub(crate) fn write_merge_ai_repo_config(
    repo: &Path,
    base_url: &str,
    extra_merge: &str,
    extra_hooks: &str,
) {
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
{extra_merge}

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

pub(crate) fn write_ai_repo_config(repo: &Path, base_url: &str, extra_hooks: &str) {
    write_merge_ai_repo_config(repo, base_url, "", extra_hooks);
}

pub(crate) fn parse_events(output: &[u8]) -> Vec<Value> {
    String::from_utf8(output.to_vec())
        .expect("utf8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("valid ndjson"))
        .collect()
}

pub(crate) fn find_tool_result<'a>(events: &'a [Value], message: &str) -> &'a Value {
    events
        .iter()
        .find(|event| {
            event["event_type"] == "tool_result" && event["message"].as_str() == Some(message)
        })
        .expect("tool result event")
}
