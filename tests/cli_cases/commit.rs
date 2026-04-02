use super::*;

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
fn commit_prunes_unknown_paths_but_keeps_deleted_files() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src")).expect("mkdir src");
    fs::write(repo.path().join("src/old.rs"), "pub fn old() {}\n").expect("write old");
    run_git(repo.path(), ["add", "."]);
    run_git(repo.path(), ["commit", "-m", "feat(core): add old file"]);

    fs::remove_file(repo.path().join("src/old.rs")).expect("delete old");

    let (_server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([commit_group(
                Some("core"),
                &["src/old.rs", "pkg/middleware/response.go"],
                "feat(core): remove old file",
                "keep real deletion and drop unknown path",
            )]),
            0.92,
        )],
        "",
        "",
        &["--json", "commit"],
    );
    assert!(output.status.success(), "commit failed: {:?}", output);

    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "commit_plan");
    let groups = result["data"]["plan"]["groups"]
        .as_array()
        .expect("groups array");
    assert_eq!(groups.len(), 1);
    let files = groups[0]["files"].as_array().expect("files");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].as_str(), Some("src/old.rs"));

    let show = StdCommand::new("git")
        .args(["show", "--name-status", "--format=", "HEAD"])
        .current_dir(repo.path())
        .output()
        .expect("git show");
    assert!(show.status.success(), "git show failed");
    let names = String::from_utf8(show.stdout).expect("utf8");
    assert!(names.contains("D\tsrc/old.rs"));
    assert!(!names.contains("pkg/middleware/response.go"));
}

#[test]
fn commit_keeps_staged_deletions_without_readding_them() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src")).expect("mkdir src");
    fs::write(repo.path().join("src/old.rs"), "pub fn old() {}\n").expect("write old");
    fs::write(repo.path().join("src/new.rs"), "pub fn new_fn() {}\n").expect("write new");
    run_git(repo.path(), ["add", "."]);
    run_git(repo.path(), ["commit", "-m", "feat(core): add files"]);

    run_git(repo.path(), ["rm", "src/old.rs"]);
    fs::write(
        repo.path().join("src/new.rs"),
        "pub fn new_fn() { println!(\"x\"); }\n",
    )
    .expect("update new");

    let (_server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([commit_group(
                Some("core"),
                &["src/old.rs", "src/new.rs"],
                "feat(core): replace old file",
                "keep staged deletion and stage new change",
            )]),
            0.92,
        )],
        "",
        "",
        &["--json", "commit"],
    );
    assert!(output.status.success(), "commit failed: {:?}", output);

    let show = StdCommand::new("git")
        .args(["show", "--name-status", "--format=", "HEAD"])
        .current_dir(repo.path())
        .output()
        .expect("git show");
    assert!(show.status.success(), "git show failed");
    let names = String::from_utf8(show.stdout).expect("utf8");
    assert!(names.contains("D\tsrc/old.rs"));
    assert!(names.contains("M\tsrc/new.rs"));
}

#[test]
fn commit_collapses_to_single_group_when_split_confidence_is_below_threshold() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::create_dir_all(repo.path().join("docs")).expect("mkdir docs");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    fs::write(repo.path().join("docs/guide.md"), "# guide\n").expect("write docs");

    let (_server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response_with_decision(
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
                    "docs(docs): update docs",
                    "docs group",
                )
            ]),
            0.78,
            Some("split"),
            Some(0.4),
        )],
        "",
        "",
        &["commit"],
    );
    assert!(
        output.status.success(),
        "commit should collapse to single group automatically: {:?}",
        output
    );

    let log = StdCommand::new("git")
        .args(["log", "--oneline", "-1"])
        .current_dir(repo.path())
        .output()
        .expect("git log");
    let subject = String::from_utf8(log.stdout).expect("utf8");
    assert!(subject.contains("feat: update auth changes"));
}

#[test]
fn commit_dry_run_previews_without_creating_commit() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");

    let head_before = StdCommand::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo.path())
        .output()
        .expect("rev-parse before");
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
        &[
            "--json",
            "commit",
            "--dry-run",
            "--intent",
            "add auth login",
        ],
    );
    assert!(output.status.success(), "dry-run failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "commit_plan");
    assert_eq!(
        result["data"]["plan"]["groups"][0]["commit_message"],
        "feat(auth): add auth login"
    );

    let head_after = StdCommand::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo.path())
        .output()
        .expect("rev-parse after");
    assert_eq!(head_before.stdout, head_after.stdout);
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

#[cfg(unix)]
#[test]
fn commit_plan_reports_scan_progress_before_ai_request() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    let server = MockAiServer::start(vec![ai_commit_plan_response(
        serde_json::json!([commit_group(
            Some("auth"),
            &["src/auth/mod.rs"],
            "feat(auth): add auth login",
            "group auth file",
        )]),
        0.92,
    )]);
    write_commit_ai_repo_config(repo.path(), server.url(), "", "");

    let path = path_with_slow_git_status(repo.path(), 1100);
    let output = run_agent_with_env(
        repo.path(),
        &["--json", "commit", "--plan"],
        &[("GIT_RAFT_API_KEY", "test-key"), ("PATH", &path)],
    );

    let events = parse_events(&output.stdout);
    assert!(
        events.iter().any(|event| {
            event["event_type"] == "phase_changed"
                && event["phase"] == "scan"
                && event["message"] == "scanning changed files"
        }),
        "expected scan phase event: {:?}",
        events
    );
    assert!(
        events.iter().any(|event| {
            event["event_type"] == "heartbeat"
                && event["phase"] == "scan"
                && event["message"] == "still scanning changed files"
        }),
        "expected scan heartbeat event: {:?}",
        events
    );
    let ai_index = events
        .iter()
        .position(|event| event["event_type"] == "ai_request_started")
        .expect("ai request started event");
    let scan_index = events
        .iter()
        .position(|event| {
            event["event_type"] == "phase_changed"
                && event["phase"] == "scan"
                && event["message"] == "scanning changed files"
        })
        .expect("scan phase index");
    assert!(
        scan_index < ai_index,
        "scan progress should happen before AI request"
    );
}

#[test]
fn commit_plan_request_uses_plan_commit_tool() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    let server = MockAiServer::start(vec![ai_commit_plan_tool_response(
        serde_json::json!([commit_group(
            Some("auth"),
            &["src/auth/mod.rs"],
            "feat(auth): add auth login",
            "group auth file",
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

    let request = first_ai_provider_request(&server);
    assert_eq!(request["tool_choice"]["type"], "function");
    assert_eq!(request["tool_choice"]["function"]["name"], "plan_commit");
    assert_eq!(request["tools"][0]["type"], "function");
    assert_eq!(request["tools"][0]["function"]["name"], "plan_commit");
    assert_eq!(request["temperature"], 0.0);
    assert_eq!(
        request["stream"], true,
        "commit planning should enable SSE streaming"
    );
}

#[test]
fn commit_plan_reuses_cached_plan_for_identical_input() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");

    let server = MockAiServer::start(vec![ai_commit_plan_tool_response(
        serde_json::json!([commit_group(
            Some("auth"),
            &["src/auth/mod.rs"],
            "feat(auth): add auth login",
            "group auth file",
        )]),
        0.92,
    )]);
    write_commit_ai_repo_config(repo.path(), server.url(), "", "");

    let first = run_agent_with_env(
        repo.path(),
        &["--json", "commit", "--plan"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        first.status.success(),
        "first commit plan failed: {:?}",
        first
    );

    drop(server);

    let second = run_agent_with_env(
        repo.path(),
        &["--json", "commit", "--plan"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        second.status.success(),
        "second commit plan should reuse cached result: {:?}",
        second
    );
    let stdout = String::from_utf8(second.stdout).expect("utf8 stdout");
    assert!(
        stdout.contains("\"tool_name\":\"commit_plan\"")
            || stdout.contains("\"event_type\":\"tool_result\"")
    );
}

#[test]
fn commit_plan_cache_ignores_recent_subject_changes_for_same_change_set() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(
        repo.path().join("src/auth/mod.rs"),
        "pub fn login_v1() {}\n",
    )
    .expect("write auth");
    run_git(repo.path(), ["add", "src/auth/mod.rs"]);
    run_git(
        repo.path(),
        ["commit", "-m", "feat(auth): add auth baseline"],
    );
    fs::write(
        repo.path().join("src/auth/mod.rs"),
        "pub fn login_v2() {}\n",
    )
    .expect("update auth");

    let server = MockAiServer::start(vec![ai_commit_plan_tool_response(
        serde_json::json!([commit_group(
            Some("auth"),
            &["src/auth/mod.rs"],
            "feat(auth): add auth login",
            "group auth file",
        )]),
        0.92,
    )]);
    write_commit_ai_repo_config(repo.path(), server.url(), "", "");

    let first = run_agent_with_env(
        repo.path(),
        &["--json", "commit", "--plan"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        first.status.success(),
        "first commit plan failed: {:?}",
        first
    );

    fs::write(repo.path().join("notes.md"), "history only\n").expect("write notes");
    run_git(repo.path(), ["add", "notes.md"]);
    run_git(
        repo.path(),
        ["commit", "-m", "chore: unrelated history update"],
    );

    drop(server);

    let second = run_agent_with_env(
        repo.path(),
        &["--json", "commit", "--plan"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        second.status.success(),
        "cache should survive recent subject changes for same change set: {:?}",
        second
    );
    let stdout = String::from_utf8(second.stdout).expect("utf8 stdout");
    assert!(stdout.contains("reusing cached commit plan"));
}

#[test]
fn commit_plan_cache_misses_when_diff_changes_on_same_files() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(
        repo.path().join("src/auth/mod.rs"),
        "pub fn login_v1() {}\n",
    )
    .expect("write auth");
    run_git(repo.path(), ["add", "src/auth/mod.rs"]);
    run_git(
        repo.path(),
        ["commit", "-m", "feat(auth): add auth baseline"],
    );
    fs::write(
        repo.path().join("src/auth/mod.rs"),
        "pub fn login_v2() {}\n",
    )
    .expect("update auth");

    let server = MockAiServer::start(vec![ai_commit_plan_tool_response(
        serde_json::json!([commit_group(
            Some("auth"),
            &["src/auth/mod.rs"],
            "feat(auth): add auth login",
            "group auth file",
        )]),
        0.92,
    )]);
    write_commit_ai_repo_config(repo.path(), server.url(), "", "");

    let first = run_agent_with_env(
        repo.path(),
        &["--json", "commit", "--plan"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        first.status.success(),
        "first commit plan failed: {:?}",
        first
    );

    fs::write(
        repo.path().join("src/auth/mod.rs"),
        "pub fn login_v3() {}\n",
    )
    .expect("change auth diff");
    drop(server);

    let second = run_agent_with_env(
        repo.path(),
        &["--json", "commit", "--plan"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(
        !second.status.success(),
        "cache should miss when diff content changes on same files: {:?}",
        second
    );
}

#[test]
fn commit_plan_request_omits_local_hint_and_requires_independent_split_groups() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::create_dir_all(repo.path().join("docs")).expect("mkdir docs");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    fs::write(repo.path().join("docs/guide.md"), "# guide\n").expect("write docs");

    let server = MockAiServer::start(vec![ai_commit_plan_response(
        serde_json::json!([
            commit_group(
                Some("auth"),
                &["src/auth/mod.rs"],
                "feat(auth): add auth login",
                "group auth file",
            ),
            commit_group(
                Some("docs"),
                &["docs/guide.md"],
                "docs(docs): document auth login",
                "group docs file",
            )
        ]),
        0.92,
    )]);
    write_commit_ai_repo_config(repo.path(), server.url(), "", "");

    let output = run_agent_with_env(
        repo.path(),
        &["--json", "commit", "--plan"],
        &[("GIT_RAFT_API_KEY", "test-key")],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);

    let request = first_ai_user_request(&server);
    let user_payload = request["user_payload"].as_object().expect("user payload");
    assert!(
        !user_payload.contains_key("local_hint_plan"),
        "commit plan request should not include local hint plan"
    );
    assert_eq!(
        request["user_payload"]["split_requirements"]["independent_commits"],
        true
    );
    assert_eq!(
        request["user_payload"]["split_requirements"]["description"],
        "Only split commits when each resulting commit can be pulled independently and still run correctly on its own."
    );
}

#[test]
fn commit_plan_request_prompt_rejects_low_signal_subjects() {
    let repo = init_repo();
    fs::write(
        repo.path().join("README.md"),
        "# git-raft\n\nUpdated usage.\n",
    )
    .expect("write readme");

    let server = MockAiServer::start(vec![ai_commit_plan_response(
        serde_json::json!([commit_group(
            Some("readme"),
            &["README.md"],
            "docs(readme): document commit planner behavior",
            "document commit planner behavior",
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

    let request = first_ai_provider_request(&server);
    let system_prompt = request["messages"][0]["content"]
        .as_str()
        .expect("system prompt");
    assert!(system_prompt.contains("add 54 lines of documentation"));
    assert!(system_prompt.contains("Do not use line counts, file counts, or raw diff stats"));
    assert!(system_prompt.contains(
        "For documentation-only changes, name the topic, command, behavior, or workflow"
    ));
}

#[test]
fn commit_plan_accepts_tool_call_response() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    let (_server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_tool_response(
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
        &["--json", "commit", "--plan"],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "commit_plan");
    assert_eq!(
        result["data"]["plan"]["groups"][0]["commit_message"],
        "feat(auth): add auth login"
    );
}

#[test]
fn commit_plan_accepts_stringified_nested_tool_arguments() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    let group = serde_json::json!({
        "scope": "auth",
        "files": ["src/auth/mod.rs"],
        "commit_message": "feat(auth): add auth login",
        "rationale": "group auth file"
    });
    let response = serde_json::json!({
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
                                    "grouping_decision": "single",
                                    "grouping_confidence": 0.92,
                                    "single_group": group.to_string(),
                                    "groups": [group.to_string()],
                                    "confidence": 0.92,
                                    "warnings": [],
                                    "auto_executable": true
                                }).to_string()
                            }
                        }
                    ]
                }
            }
        ]
    });
    let (_server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![response],
        "",
        "",
        &["--json", "commit", "--plan"],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);
    let events = parse_events(&output.stdout);
    let result = find_tool_result(&events, "commit_plan");
    assert_eq!(
        result["data"]["plan"]["groups"][0]["commit_message"],
        "feat(auth): add auth login"
    );
}

#[test]
fn commit_plan_fails_when_ai_response_omits_plan_commit_tool_call() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join("src/auth")).expect("mkdir auth");
    fs::write(repo.path().join("src/auth/mod.rs"), "pub fn login() {}\n").expect("write auth");
    let (_server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_text_response(
            &serde_json::json!({
                "grouping_decision": "single",
                "grouping_confidence": 0.92,
                "single_group": {
                    "scope": "auth",
                    "files": ["src/auth/mod.rs"],
                    "commit_message": "feat(auth): add auth login",
                    "rationale": "group auth file"
                },
                "groups": [{
                    "scope": "auth",
                    "files": ["src/auth/mod.rs"],
                    "commit_message": "feat(auth): add auth login",
                    "rationale": "group auth file"
                }],
                "confidence": 0.92,
                "warnings": [],
                "auto_executable": true
            })
            .to_string(),
        )],
        "",
        "",
        &["--json", "commit", "--plan"],
    );
    assert!(
        !output.status.success(),
        "plain text commit plan response should be rejected: {:?}",
        output
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("plan_commit tool call") || stdout.contains("tool call"));
}

#[test]
fn commit_plan_human_output_shows_summary_not_raw_json() {
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
        &["commit", "--plan"],
    );
    assert!(output.status.success(), "commit plan failed: {:?}", output);
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("Commit plan"));
    assert!(stdout.contains("Commit 1"));
    assert!(stdout.contains("feat(auth): add auth login"));
    assert!(stdout.contains("src/auth/mod.rs"));
    assert!(!stdout.contains("\"plan\":"));
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
fn commit_plan_keeps_non_tool_dot_directories() {
    let repo = init_repo();
    fs::create_dir_all(repo.path().join(".github/workflows")).expect("mkdir workflows");
    fs::write(
        repo.path().join(".github/workflows/ci.yml"),
        "name: ci\non: [push]\n",
    )
    .expect("write workflow");

    let (server, output) = run_commit_with_mock_ai(
        repo.path(),
        vec![ai_commit_plan_response(
            serde_json::json!([commit_group(
                Some("ci"),
                &[".github/workflows/ci.yml"],
                "ci: add workflow",
                "group workflow file",
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
    let files = groups[0]["files"]
        .as_array()
        .expect("files")
        .iter()
        .map(|file| file.as_str().expect("file").to_string())
        .collect::<Vec<_>>();
    assert_eq!(files, vec![".github/workflows/ci.yml".to_string()]);
    let request = first_ai_user_request(&server);
    let changed_files = request["user_payload"]["changed_files"]
        .as_array()
        .expect("changed files")
        .iter()
        .map(|file| file.as_str().expect("file").to_string())
        .collect::<Vec<_>>();
    assert_eq!(changed_files, vec![".github/workflows/ci.yml".to_string()]);
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
        &["--json", "commit", "--plan", "--lang", "zh"],
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
                "feat(auth): update auth changes",
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
    assert_eq!(message, "feat(auth): update auth changes");
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
                "feat(auth): update auth changes\n\nFiles: src/auth/mod.rs",
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
