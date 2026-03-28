use crate::ai::{AiClient, AiPatch, collect_repo_context};
use crate::commands::ai_tasks::run_ai_hook;
use crate::config;
use crate::events::Emitter;
use crate::git::{self, GitExec};
use crate::store::{RunStatus, RunStore};
use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use std::path::PathBuf;

use super::retention::{ConflictTextFile, validate_patch};
use super::validation::{
    NON_TEXT_CONFLICT, ValidationAttemptRecord, ValidationTrace, run_validation_commands,
};

pub(crate) async fn resolve_conflicts_with_ai(
    git: &GitExec,
    command: &str,
    cwd: &PathBuf,
    repo: Option<&git::RepoContext>,
    resolved_config: &config::ResolvedConfig,
    conflicts: &[String],
    store: Option<&RunStore>,
    emitter: &mut Emitter,
) -> Result<bool> {
    let config = AiClient::config_from_repo(repo)?;
    if config.base_url.trim().is_empty() {
        emitter
            .emit(
                "phase_changed",
                Some("ai_wait"),
                Some("provider not configured, leaving conflicts for manual review".to_string()),
                None,
            )
            .await?;
        return Ok(false);
    }
    let client = AiClient::from_repo(repo)?;
    let Some(repo_ctx) = repo else {
        return Ok(false);
    };

    let conflict_files = match load_conflict_files(git, conflicts).await {
        Ok(files) => files,
        Err(reason) => {
            let record = ValidationAttemptRecord::non_text_failure(reason.clone());
            persist_validation(
                store,
                ValidationTrace {
                    attempts: vec![record],
                },
            )?;
            if let Some(store) = store {
                store.finish(RunStatus::Failed, None, Some(false))?;
            }
            emitter
                .emit(
                    "awaiting_confirmation",
                    Some("review"),
                    Some(reason.clone()),
                    Some(json!({
                        "attempt": 0,
                        "validationPassed": false,
                        "rejectionReason": reason,
                    })),
                )
                .await?;
            return Ok(false);
        }
    };

    let max_attempts = resolved_config
        .merge
        .repair_attempts
        .saturating_add(1)
        .max(1);
    let mut trace = ValidationTrace::default();
    let mut repair_context = None;

    for attempt in 1..=max_attempts {
        let patch = request_conflict_patch(
            &client,
            git,
            command,
            cwd,
            repo_ctx,
            resolved_config,
            conflicts,
            attempt,
            repair_context.clone(),
            store,
            emitter,
        )
        .await?;

        let retention = validate_patch(&conflict_files, &patch);
        let attempt_record = if retention.passed {
            run_validation_commands(
                repo_ctx.root_dir.as_path(),
                &patch,
                &resolved_config.merge.verification,
                attempt,
            )
            .await?
        } else {
            ValidationAttemptRecord::from_retention(attempt, &retention)
        };

        trace.attempts.push(attempt_record.clone());
        persist_validation(store, trace.clone())?;
        emitter
            .emit(
                "verify_finished",
                Some("verify"),
                Some("validated AI merge candidate".to_string()),
                Some(json!({
                    "attempt": attempt,
                    "success": attempt_record.validation_passed,
                    "validationPassed": attempt_record.validation_passed,
                    "rejectionReason": attempt_record.rejection_reason,
                    "commandCount": attempt_record.commands.len(),
                })),
            )
            .await?;

        if attempt_record.validation_passed {
            let snapshot = git.inspect_snapshot().await.unwrap_or_default();
            let request_summary = json!({
                "task": "resolve_conflicts",
                "attempt": attempt,
                "hasRepoContext": true,
                "conflictFiles": patch.files.iter().map(|file| file.path.clone()).collect::<Vec<_>>(),
                "validationPassed": true,
            });
            let response_summary = json!({
                "task": "resolve_conflicts",
                "attempt": attempt,
                "kind": "patch",
                "patchConfidence": patch.confidence,
                "fileCount": patch.files.len(),
                "validationPassed": true,
                "rejectionReason": Value::Null,
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

            apply_patch(git, &patch, emitter).await?;
            if let Some(store) = store {
                store.finish(RunStatus::Succeeded, None, Some(true))?;
            }
            emitter
                .emit(
                    "ai_patch_applied",
                    Some("exec"),
                    Some("applied conflict patch".to_string()),
                    Some(json!({
                        "attempt": attempt,
                        "confidence": patch.confidence,
                        "files": patch.files.iter().map(|file| file.path.clone()).collect::<Vec<_>>(),
                        "validationPassed": true,
                    })),
                )
                .await?;
            return Ok(true);
        }

        repair_context = Some(attempt_record.repair_context(&patch));
        if attempt < max_attempts && attempt_record.is_repairable() {
            continue;
        }

        if let Some(store) = store {
            store.finish(RunStatus::Failed, None, Some(false))?;
        }
        emitter
            .emit(
                "awaiting_confirmation",
                Some("review"),
                Some(
                    attempt_record
                        .rejection_reason
                        .clone()
                        .unwrap_or_else(|| "AI merge candidate requires manual review".to_string()),
                ),
                Some(json!({
                    "attempt": attempt,
                    "validationPassed": false,
                    "rejectionReason": attempt_record.rejection_reason,
                })),
            )
            .await?;
        return Ok(false);
    }

    Ok(false)
}

async fn request_conflict_patch(
    client: &AiClient,
    git: &GitExec,
    command: &str,
    cwd: &PathBuf,
    repo: &git::RepoContext,
    resolved_config: &config::ResolvedConfig,
    conflicts: &[String],
    attempt: usize,
    repair_context: Option<Value>,
    store: Option<&RunStore>,
    emitter: &mut Emitter,
) -> Result<AiPatch> {
    let repo_context = Some(collect_repo_context(git, repo, client.config()).await);
    let request = client
        .build_conflict_request(git, conflicts, repo_context, attempt, repair_context)
        .await?;

    let snapshot = git.inspect_snapshot().await.unwrap_or_default();
    let request_summary = request.summary();
    run_ai_hook(
        "beforeAiRequest",
        command,
        cwd,
        repo,
        resolved_config,
        &snapshot,
        Some(request.task_name()),
        Some(&request_summary),
        None,
        None,
        emitter,
    )
    .await?;

    let exchange = client.execute(request, emitter, store).await?;
    let snapshot = git.inspect_snapshot().await.unwrap_or_default();
    let request_summary = exchange.request_summary();
    let response_summary = exchange.response_summary();
    run_ai_hook(
        "afterAiResponse",
        command,
        cwd,
        repo,
        resolved_config,
        &snapshot,
        Some(exchange.task_name()),
        Some(&request_summary),
        Some(&response_summary),
        exchange.patch_confidence(),
        emitter,
    )
    .await?;

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
                "attempt": attempt,
                "confidence": patch.confidence,
                "files": patch.files.iter().map(|file| file.path.clone()).collect::<Vec<_>>(),
            })),
        )
        .await?;
    Ok(patch)
}

async fn load_conflict_files(
    git: &GitExec,
    conflicts: &[String],
) -> Result<Vec<ConflictTextFile>, String> {
    let mut files = Vec::new();
    for path in conflicts {
        let _base = git
            .read_stage_file(1, path)
            .await
            .map_err(|_| format!("{NON_TEXT_CONFLICT}: {path}"))?;
        let _ours = git
            .read_stage_file(2, path)
            .await
            .map_err(|_| format!("{NON_TEXT_CONFLICT}: {path}"))?;
        let _theirs = git
            .read_stage_file(3, path)
            .await
            .map_err(|_| format!("{NON_TEXT_CONFLICT}: {path}"))?;
        let current = git
            .read_worktree_file(path)
            .await
            .map_err(|_| format!("{NON_TEXT_CONFLICT}: {path}"))?;
        files.push(ConflictTextFile {
            path: path.clone(),
            current,
        });
    }
    Ok(files)
}

async fn apply_patch(git: &GitExec, patch: &AiPatch, emitter: &mut Emitter) -> Result<()> {
    for file in &patch.files {
        git.write_file(&file.path, &file.resolved_content).await?;
        git.run(&["add".to_string(), file.path.clone()], emitter)
            .await?;
    }
    let unresolved = git.unresolved_conflicts().await?;
    if !unresolved.is_empty() {
        return Err(anyhow!("conflicts remain after applying AI patch"));
    }
    Ok(())
}

fn persist_validation(store: Option<&RunStore>, trace: ValidationTrace) -> Result<()> {
    if let Some(store) = store {
        store.write_json("validation.json", &trace)?;
    }
    Ok(())
}
