use crate::ai::{AiClient, collect_repo_context};
use crate::commit::{self, collect_planning_inputs};
use crate::config;
use crate::events::Emitter;
use crate::git::{self, GitExec};
use crate::hooks::{self, HookContext, run_hooks};
use crate::store::{RunStatus, RunStore};
use anyhow::{Result, anyhow};
use serde_json::json;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use super::ai_tasks::run_ai_hook;

pub(crate) struct CommitRun {
    pub(crate) plan_only: bool,
    pub(crate) dry_run: bool,
    pub(crate) intent: Option<String>,
    pub(crate) language: Option<String>,
    pub(crate) args: Vec<String>,
    pub(crate) resolved_config: config::ResolvedConfig,
}

pub(crate) async fn run_commit(
    request: CommitRun,
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    store: Option<RunStore>,
    emitter: &mut Emitter,
) -> Result<()> {
    let CommitRun {
        plan_only,
        dry_run,
        intent,
        language,
        args: _args,
        mut resolved_config,
    } = request;
    if let Some(language) = language {
        resolved_config.commit.language = language;
    }
    let repo_ctx = repo.ok_or_else(|| anyhow!("commit requires a git repository"))?;
    let git = GitExec::new(cwd.clone(), Some(repo_ctx.clone()));
    let snapshot = git.inspect_snapshot().await?;
    let planning_inputs = collect_planning_inputs(&snapshot, &resolved_config);
    let change_set_fingerprint =
        compute_commit_change_set_fingerprint(&repo_ctx.root_dir, &planning_inputs)?;
    let client = AiClient::from_repo(Some(&repo_ctx))?;
    let mut ai_context_config = client.config().clone();
    ai_context_config.commit_format = resolved_config.commit.format.clone();
    ai_context_config.commit_language = resolved_config.commit.language.clone();
    ai_context_config.commit_use_gitmoji = resolved_config.commit.use_gitmoji;
    ai_context_config.commit_include_body = resolved_config.commit.include_body;
    ai_context_config.commit_include_footer = resolved_config.commit.include_footer;
    ai_context_config.commit_ignore_paths = resolved_config.commit.ignore_paths.clone();
    let repo_context = Some(collect_repo_context(&git, &repo_ctx, &ai_context_config).await);
    let request = client.build_commit_request(
        planning_inputs,
        intent.clone(),
        &resolved_config.commit,
        repo_context,
    );
    let request_summary = request.summary();
    let cache_key = request.cache_fingerprint(
        client.config().base_url.as_str(),
        client.config().model.as_str(),
        &change_set_fingerprint,
    )?;
    let cached_plan = load_cached_commit_plan(&repo_ctx.git_dir, &cache_key)?;
    let plan = if let Some(plan) = cached_plan {
        emitter
            .emit(
                "phase_changed",
                Some("ai_wait"),
                Some("reusing cached commit plan".to_string()),
                Some(json!({ "task": request.task_name(), "cache_key": cache_key })),
            )
            .await?;
        plan
    } else {
        run_ai_hook(
            "beforeAiRequest",
            "commit",
            &cwd,
            &repo_ctx,
            &resolved_config,
            &snapshot,
            Some(request.task_name()),
            Some(&request_summary),
            None,
            None,
            emitter,
        )
        .await?;

        let exchange = client.execute(request, emitter, store.as_ref()).await?;
        let response_summary = exchange.response_summary();
        run_ai_hook(
            "afterAiResponse",
            "commit",
            &cwd,
            &repo_ctx,
            &resolved_config,
            &snapshot,
            Some(exchange.task_name()),
            Some(&request_summary),
            Some(&response_summary),
            exchange.patch_confidence(),
            emitter,
        )
        .await?;
        let plan = exchange
            .into_commit_plan()?
            .normalize_for_execution(&resolved_config);
        store_cached_commit_plan(&repo_ctx.git_dir, &cache_key, &plan)?;
        plan
    };

    let hook = run_hooks(HookContext {
        event: "afterCommitPlan",
        command: "commit",
        repo: &repo_ctx,
        cwd: &cwd,
        config: &resolved_config,
        git_snapshot: &snapshot,
        intent: intent.as_deref(),
        commit_plan: Some(&plan),
        commit_group: None,
        commit_message: None,
        agent_task: None,
        agent_request_summary: None,
        agent_response_summary: None,
        patch_confidence: None,
    })
    .await?;

    emitter
        .emit(
            "tool_result",
            Some("done"),
            Some("commit_plan".to_string()),
            Some(json!({
                "plan": &plan,
                "blocked": hook.blocked,
                "reason": hook.reason.clone(),
                "warnings": hook.warnings.clone(),
            })),
        )
        .await?;

    if !emitter.json_mode() {
        println!("{}", render_commit_plan_summary(&plan, &hook));
    }

    if hook.blocked {
        emitter
            .emit(
                "commandFailed",
                Some("plan"),
                Some(
                    hook.reason
                        .unwrap_or_else(|| "hook blocked commit plan".to_string()),
                ),
                Some(json!({ "blocked": true })),
            )
            .await?;
        return Err(anyhow!("hook blocked commit plan"));
    }

    if plan_only || dry_run {
        return Ok(());
    }

    for group in &plan.groups {
        let group_hook = run_hooks(HookContext {
            event: "beforeGroupCommit",
            command: "commit",
            repo: &repo_ctx,
            cwd: &cwd,
            config: &resolved_config,
            git_snapshot: &snapshot,
            intent: intent.as_deref(),
            commit_plan: Some(&plan),
            commit_group: Some(group),
            commit_message: Some(&group.commit_message),
            agent_task: None,
            agent_request_summary: None,
            agent_response_summary: None,
            patch_confidence: None,
        })
        .await?;
        if group_hook.blocked {
            emitter
                .emit(
                    "commandFailed",
                    Some("exec"),
                    Some(
                        group_hook
                            .reason
                            .unwrap_or_else(|| "hook blocked commit group".to_string()),
                    ),
                    Some(json!({ "blocked": true })),
                )
                .await?;
            return Err(anyhow!("hook blocked commit group"));
        }
        let message = group_hook
            .commit_message
            .as_deref()
            .unwrap_or(&group.commit_message)
            .to_string();
        git.stage_files(&group.files).await?;
        git.create_commit(&message).await?;
        let after_snapshot = git.inspect_snapshot().await?;
        let _ = run_hooks(HookContext {
            event: "afterGroupCommit",
            command: "commit",
            repo: &repo_ctx,
            cwd: &cwd,
            config: &resolved_config,
            git_snapshot: &after_snapshot,
            intent: intent.as_deref(),
            commit_plan: Some(&plan),
            commit_group: Some(group),
            commit_message: Some(&message),
            agent_task: None,
            agent_request_summary: None,
            agent_response_summary: None,
            patch_confidence: None,
        })
        .await;
    }

    if let Some(store) = &store {
        store.finish(RunStatus::Succeeded, None, Some(true))?;
    }
    Ok(())
}

fn commit_plan_cache_dir(git_dir: &Path) -> PathBuf {
    git_dir.join("git-raft").join("commit-plan-cache")
}

fn compute_commit_change_set_fingerprint(
    root_dir: &Path,
    planning_inputs: &commit::CommitPlanningInputs,
) -> Result<String> {
    let file_states = planning_inputs
        .changed_files
        .iter()
        .map(|path| {
            let full_path = root_dir.join(path);
            let state = if full_path.exists() {
                let bytes = fs::read(&full_path)?;
                json!({
                    "path": path,
                    "state": "present",
                    "content_hash": hash_bytes(&bytes),
                })
            } else {
                json!({
                    "path": path,
                    "state": "deleted",
                })
            };
            Ok(state)
        })
        .collect::<Result<Vec<_>>>()?;
    let material = serde_json::to_vec(&json!({
        "changed_files": planning_inputs.changed_files,
        "staged_files": planning_inputs.staged_files,
        "unstaged_files": planning_inputs.unstaged_files,
        "untracked_files": planning_inputs.untracked_files,
        "file_states": file_states,
    }))?;
    Ok(hash_bytes(&material))
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn commit_plan_cache_path(git_dir: &Path, cache_key: &str) -> PathBuf {
    commit_plan_cache_dir(git_dir).join(format!("{cache_key}.json"))
}

fn load_cached_commit_plan(git_dir: &Path, cache_key: &str) -> Result<Option<commit::CommitPlan>> {
    let path = commit_plan_cache_path(git_dir, cache_key);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

fn store_cached_commit_plan(
    git_dir: &Path,
    cache_key: &str,
    plan: &commit::CommitPlan,
) -> Result<()> {
    let dir = commit_plan_cache_dir(git_dir);
    fs::create_dir_all(&dir)?;
    fs::write(
        commit_plan_cache_path(git_dir, cache_key),
        serde_json::to_vec_pretty(plan)?,
    )?;
    Ok(())
}

pub(crate) fn render_commit_plan_summary(
    plan: &commit::CommitPlan,
    hook: &hooks::HookDecision,
) -> String {
    let mut out = String::new();
    out.push_str("Commit plan\n");
    out.push_str(&format!("decision: {:?}\n", plan.grouping_decision).to_lowercase());
    out.push_str(&format!("confidence: {:.2}\n", plan.confidence));
    out.push_str(&format!(
        "grouping confidence: {:.2}\n",
        plan.grouping_confidence
    ));
    if !plan.warnings.is_empty() || !hook.warnings.is_empty() {
        out.push_str("warnings:\n");
        for warning in &plan.warnings {
            out.push_str("- ");
            out.push_str(warning);
            out.push('\n');
        }
        for warning in &hook.warnings {
            out.push_str("- ");
            out.push_str(warning);
            out.push('\n');
        }
    }
    for (index, group) in plan.groups.iter().enumerate() {
        out.push_str(&format!("Commit {}\n", index + 1));
        out.push_str("message: ");
        out.push_str(&group.commit_message);
        out.push('\n');
        out.push_str("rationale: ");
        out.push_str(&group.rationale);
        out.push('\n');
        out.push_str("files:\n");
        for file in &group.files {
            out.push_str("- ");
            out.push_str(file);
            out.push('\n');
        }
    }
    if let Some(reason) = &hook.reason {
        out.push_str("blocked reason: ");
        out.push_str(reason);
        out.push('\n');
    }
    out.trim_end().to_string()
}
