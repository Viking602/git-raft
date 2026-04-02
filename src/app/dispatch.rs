use crate::cli::{Cli, CommandKind, CommitLanguageArg};
use crate::commands::author::{AuthorRun, run_author};
use crate::commands::branch::{BranchRun, run_branch};
use crate::commands::commit::{CommitRun, run_commit};
use crate::commands::merge_rebase::{MergeRun, run_merge_like};
use crate::config;
use crate::events::Emitter;
use crate::git::{self, GitExec};
use crate::hooks::{HookContext, run_hooks};
use crate::risk::{RiskLevel, classify};
use crate::store::{RunStatus, RunStore};
use anyhow::{Result, anyhow};
use serde_json::json;
use std::path::PathBuf;
use uuid::Uuid;

pub(crate) async fn run_cli(cli: Cli, cwd: PathBuf) -> Result<()> {
    let run_id = Uuid::new_v4();
    let repo = GitExec::discover_repo(&cwd).await?;
    let resolved_config = config::resolve_config(repo.as_ref().map(|repo| repo.root_dir.as_path()))
        .map(|(config, _)| config)
        .unwrap_or_default();
    let store = repo
        .as_ref()
        .map(|repo| RunStore::create(repo.git_dir.clone(), run_id, cli.command.label()))
        .transpose()?;
    let mut emitter = Emitter::new(cli.json, run_id, store.clone());

    emitter
        .emit(
            "run_started",
            Some("sense"),
            Some(format!("starting {}", cli.command.label())),
            Some(json!({
                "command": cli.command.label(),
                "cwd": cwd.display().to_string(),
                "inside_repo": repo.is_some(),
            })),
        )
        .await?;

    let result =
        dispatch_command(cli, cwd, repo, resolved_config, store.clone(), &mut emitter).await;

    match result {
        Ok(()) => {
            if let Some(store) = &store {
                store.finish(RunStatus::Succeeded, None, None)?;
            }
            emitter
                .emit(
                    "run_finished",
                    Some("done"),
                    Some("run completed".to_string()),
                    Some(json!({ "ok": true })),
                )
                .await?;
            Ok(())
        }
        Err(err) => {
            if let Some(store) = &store {
                store.finish(RunStatus::Failed, None, None)?;
            }
            emitter
                .emit(
                    "commandFailed",
                    Some("error"),
                    Some(err.to_string()),
                    Some(json!({ "blocked": false })),
                )
                .await?;
            emitter
                .emit(
                    "run_finished",
                    Some("error"),
                    Some(err.to_string()),
                    Some(json!({ "ok": false })),
                )
                .await?;
            Err(err)
        }
    }
}

async fn dispatch_command(
    cli: Cli,
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    resolved_config: config::ResolvedConfig,
    store: Option<RunStore>,
    emitter: &mut Emitter,
) -> Result<()> {
    let risk = classify(&cli.command);
    if risk.level == RiskLevel::High && !cli.yes {
        emitter
            .emit(
                "risk_detected",
                Some("plan"),
                Some(risk.reason.to_string()),
                Some(json!({
                    "level": risk.level.as_str(),
                    "command": cli.command.label(),
                })),
            )
            .await?;
        emitter
            .emit(
                "awaiting_confirmation",
                Some("plan"),
                Some(format!(
                    "rerun with --yes to confirm {}",
                    cli.command.label()
                )),
                None,
            )
            .await?;
        return Err(anyhow!("confirmation required for {}", cli.command.label()));
    }

    let git = GitExec::new(cwd.clone(), repo.clone());
    let snapshot = if repo.is_some() {
        git.inspect_snapshot().await.unwrap_or_default()
    } else {
        git::GitSnapshot::default()
    };
    if let Some(repo_ctx) = repo.as_ref() {
        let hook = run_hooks(HookContext {
            event: "beforeCommand",
            command: cli.command.label(),
            repo: repo_ctx,
            cwd: &cwd,
            config: &resolved_config,
            git_snapshot: &snapshot,
            intent: None,
            commit_plan: None,
            commit_group: None,
            commit_message: None,
            agent_task: None,
            agent_request_summary: None,
            agent_response_summary: None,
            patch_confidence: None,
        })
        .await?;
        if hook.blocked {
            emitter
                .emit(
                    "commandFailed",
                    Some("plan"),
                    Some(
                        hook.reason
                            .unwrap_or_else(|| "hook blocked command".to_string()),
                    ),
                    Some(json!({ "blocked": true })),
                )
                .await?;
            return Err(anyhow!("hook blocked command"));
        }
    }

    let command_label = cli.command.label().to_string();
    let result = match cli.command {
        CommandKind::Commit {
            plan,
            dry_run,
            intent,
            lang,
            args,
        } => {
            run_commit(
                CommitRun {
                    plan_only: plan,
                    dry_run,
                    intent,
                    language: lang.map(CommitLanguageArg::as_str).map(str::to_string),
                    args,
                    resolved_config: resolved_config.clone(),
                },
                cwd.clone(),
                repo.clone(),
                store.clone(),
                emitter,
            )
            .await
        }
        CommandKind::Branch { name, target } => {
            run_branch(
                BranchRun { name, target },
                cwd.clone(),
                repo.clone(),
                emitter,
            )
            .await
        }
        CommandKind::Merge {
            target,
            args,
            apply_ai,
        } => {
            run_merge_like(
                MergeRun {
                    mode: "merge".to_string(),
                    target,
                    args,
                    apply_ai,
                },
                resolved_config.clone(),
                cwd.clone(),
                repo.clone(),
                store.clone(),
                emitter,
            )
            .await
        }
        CommandKind::Rebase {
            target,
            args,
            apply_ai,
        } => {
            run_merge_like(
                MergeRun {
                    mode: "rebase".to_string(),
                    target,
                    args,
                    apply_ai,
                },
                resolved_config.clone(),
                cwd.clone(),
                repo.clone(),
                store.clone(),
                emitter,
            )
            .await
        }
        CommandKind::Author {
            name,
            email,
            force,
            push,
        } => {
            run_author(
                AuthorRun {
                    name,
                    email,
                    force,
                    push,
                },
                cwd.clone(),
                repo.clone(),
                emitter,
            )
            .await
        }
    };

    if result.is_ok()
        && let Some(repo_ctx) = repo.as_ref()
    {
        let snapshot = git.inspect_snapshot().await.unwrap_or_default();
        let _ = run_hooks(HookContext {
            event: "afterCommand",
            command: &command_label,
            repo: repo_ctx,
            cwd: &cwd,
            config: &resolved_config,
            git_snapshot: &snapshot,
            intent: None,
            commit_plan: None,
            commit_group: None,
            commit_message: None,
            agent_task: None,
            agent_request_summary: None,
            agent_response_summary: None,
            patch_confidence: None,
        })
        .await;
    }
    result
}
