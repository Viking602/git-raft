use crate::ai::{AiClient, AiPatch, collect_repo_context};
use crate::config;
use crate::events::Emitter;
use crate::git::{self, GitExec};
use crate::hooks::{HookContext, run_hooks};
use crate::store::{RunStatus, RunStore};
use anyhow::{Result, anyhow};
use serde_json::json;
use std::path::PathBuf;

pub(crate) async fn run_ask(
    prompt: String,
    cwd: PathBuf,
    repo: Option<&git::RepoContext>,
    resolved_config: config::ResolvedConfig,
    store: Option<RunStore>,
    emitter: &mut Emitter,
) -> Result<()> {
    if prompt.trim().is_empty() {
        return Err(anyhow!("ask requires a prompt"));
    }
    let client = AiClient::from_repo(repo)?;
    let repo_context = if let Some(repo_ctx) = repo {
        let git = GitExec::new(cwd.clone(), Some(repo_ctx.clone()));
        Some(collect_repo_context(&git, repo_ctx, client.config()).await)
    } else {
        None
    };
    let request = client.build_ask_request(prompt, repo_context);
    if let Some(repo_ctx) = repo {
        let git = GitExec::new(cwd.clone(), Some(repo_ctx.clone()));
        let snapshot = git.inspect_snapshot().await.unwrap_or_default();
        let request_summary = request.summary();
        run_ai_hook(
            "beforeAiRequest",
            "ask",
            &cwd,
            repo_ctx,
            &resolved_config,
            &snapshot,
            Some(request.task_name()),
            Some(&request_summary),
            None,
            None,
            emitter,
        )
        .await?;
    }

    let exchange = client.execute(request, emitter, store.as_ref()).await?;
    if let Some(repo_ctx) = repo {
        let git = GitExec::new(cwd.clone(), Some(repo_ctx.clone()));
        let snapshot = git.inspect_snapshot().await.unwrap_or_default();
        let request_summary = exchange.request_summary();
        let response_summary = exchange.response_summary();
        run_ai_hook(
            "afterAiResponse",
            "ask",
            &cwd,
            repo_ctx,
            &resolved_config,
            &snapshot,
            Some(exchange.task_name()),
            Some(&request_summary),
            Some(&response_summary),
            exchange.patch_confidence(),
            emitter,
        )
        .await?;
    }
    let reply = exchange.into_text()?;
    emitter
        .emit(
            "verify_finished",
            Some("verify"),
            Some("received ai response".to_string()),
            Some(json!({ "success": true })),
        )
        .await?;
    if emitter.json_mode() {
        emitter
            .emit(
                "tool_result",
                Some("ai_wait"),
                Some("ask result".to_string()),
                Some(json!({ "text": reply })),
            )
            .await?;
    } else {
        println!("{reply}");
    }
    Ok(())
}

pub(crate) async fn run_ai_hook(
    event: &str,
    command: &str,
    cwd: &PathBuf,
    repo: &git::RepoContext,
    resolved_config: &config::ResolvedConfig,
    git_snapshot: &git::GitSnapshot,
    agent_task: Option<&str>,
    agent_request_summary: Option<&serde_json::Value>,
    agent_response_summary: Option<&serde_json::Value>,
    patch_confidence: Option<f32>,
    emitter: &mut Emitter,
) -> Result<()> {
    let decision = run_hooks(HookContext {
        event,
        command,
        repo,
        cwd,
        config: resolved_config,
        git_snapshot,
        intent: None,
        commit_plan: None,
        commit_group: None,
        commit_message: None,
        agent_task,
        agent_request_summary,
        agent_response_summary,
        patch_confidence,
    })
    .await?;
    if decision.blocked {
        let reason = decision
            .reason
            .unwrap_or_else(|| format!("hook blocked {event}"));
        emitter
            .emit(
                "commandFailed",
                Some("ai_wait"),
                Some(reason.clone()),
                Some(json!({ "blocked": true, "event": event })),
            )
            .await?;
        return Err(anyhow!(reason));
    }
    Ok(())
}

pub(crate) async fn attempt_ai_resolution(
    git: &GitExec,
    command: &str,
    cwd: &PathBuf,
    repo: Option<&git::RepoContext>,
    resolved_config: &config::ResolvedConfig,
    conflicts: &[String],
    store: Option<&RunStore>,
    emitter: &mut Emitter,
) -> Result<Option<AiPatch>> {
    let client = match AiClient::config_from_repo(repo) {
        Ok(_) => AiClient::from_repo(repo)?,
        Err(_) => {
            emitter
                .emit(
                    "phase_changed",
                    Some("ai_wait"),
                    Some(
                        "provider not configured, leaving conflicts for manual review".to_string(),
                    ),
                    None,
                )
                .await?;
            return Ok(None);
        }
    };

    let repo_context = match repo {
        Some(repo_ctx) => Some(collect_repo_context(git, repo_ctx, client.config()).await),
        None => None,
    };
    let request = client
        .build_conflict_request(git, conflicts, repo_context)
        .await?;

    if let Some(repo_ctx) = repo {
        let snapshot = git.inspect_snapshot().await.unwrap_or_default();
        let request_summary = request.summary();
        run_ai_hook(
            "beforeAiRequest",
            command,
            cwd,
            repo_ctx,
            resolved_config,
            &snapshot,
            Some(request.task_name()),
            Some(&request_summary),
            None,
            None,
            emitter,
        )
        .await?;
    }

    let exchange = client.execute(request, emitter, store).await?;
    if let Some(repo_ctx) = repo {
        let snapshot = git.inspect_snapshot().await.unwrap_or_default();
        let request_summary = exchange.request_summary();
        let response_summary = exchange.response_summary();
        run_ai_hook(
            "afterAiResponse",
            command,
            cwd,
            repo_ctx,
            resolved_config,
            &snapshot,
            Some(exchange.task_name()),
            Some(&request_summary),
            Some(&response_summary),
            exchange.patch_confidence(),
            emitter,
        )
        .await?;
    }

    let patch = exchange.into_patch()?;
    if let Some(store) = store {
        store.write_json("patch.json", &patch)?;
    }
    emitter
        .emit(
            "ai_patch_ready",
            Some("ai_wait"),
            Some("conflict patch generated".to_string()),
            Some(json!({
                "confidence": patch.confidence,
                "files": patch.files.iter().map(|file| file.path.clone()).collect::<Vec<_>>(),
            })),
        )
        .await?;
    Ok(Some(patch))
}

pub(crate) async fn maybe_apply_patch(
    git: &GitExec,
    command: &str,
    cwd: &PathBuf,
    repo: Option<&git::RepoContext>,
    resolved_config: &config::ResolvedConfig,
    patch: Option<AiPatch>,
    store: Option<RunStore>,
    emitter: &mut Emitter,
) -> Result<()> {
    let Some(patch) = patch else {
        return Ok(());
    };
    if patch.confidence < 0.75 {
        emitter
            .emit(
                "awaiting_confirmation",
                Some("review"),
                Some("ai confidence below 0.75; patch saved but not applied".to_string()),
                Some(json!({ "confidence": patch.confidence })),
            )
            .await?;
        return Ok(());
    }

    if let Some(repo_ctx) = repo {
        let snapshot = git.inspect_snapshot().await.unwrap_or_default();
        let request_summary = json!({
            "task": "resolve_conflicts",
            "hasRepoContext": true,
            "conflictFiles": patch.files.iter().map(|file| file.path.clone()).collect::<Vec<_>>(),
        });
        let response_summary = json!({
            "task": "resolve_conflicts",
            "kind": "patch",
            "patchConfidence": patch.confidence,
            "fileCount": patch.files.len(),
        });
        run_ai_hook(
            "beforePatchApply",
            command,
            cwd,
            repo_ctx,
            resolved_config,
            &snapshot,
            Some("resolve_conflicts"),
            Some(&request_summary),
            Some(&response_summary),
            Some(patch.confidence),
            emitter,
        )
        .await?;
    }

    for file in &patch.files {
        git.write_file(&file.path, &file.resolved_content).await?;
        git.run(&["add".to_string(), file.path.clone()], emitter)
            .await?;
    }
    emitter
        .emit(
            "ai_patch_applied",
            Some("exec"),
            Some("applied conflict patch".to_string()),
            Some(json!({
                "confidence": patch.confidence,
                "files": patch.files.iter().map(|file| file.path.clone()).collect::<Vec<_>>(),
            })),
        )
        .await?;

    let unresolved = git.unresolved_conflicts().await?;
    let diff_clean = git.diff_check().await?;
    let success = unresolved.is_empty() && diff_clean;
    if let Some(store) = store {
        store.finish(
            if success {
                RunStatus::Succeeded
            } else {
                RunStatus::Failed
            },
            None,
            Some(success),
        )?;
    }
    emitter
        .emit(
            "verify_finished",
            Some("verify"),
            Some("checked conflict resolution output".to_string()),
            Some(json!({
                "success": success,
                "remaining_conflicts": unresolved,
                "diff_check": diff_clean,
            })),
        )
        .await?;
    Ok(())
}
