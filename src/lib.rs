mod ai;
mod cli;
mod commit;
mod config;
mod events;
mod git;
mod hooks;
mod risk;
mod store;

use ai::{AiClient, AiPatch};
use anyhow::{Context, Result, anyhow};
use clap::Parser;
use cli::{Cli, CommandKind, ConfigCommand, ConfigScopeArg, ConfigWritableScopeArg, ScopesCommand};
use commit::{build_plan, generate_scopes, list_scopes};
use config::{ConfigKey, ConfigScope};
use events::Emitter;
use git::GitExec;
use hooks::{HookContext, run_hooks};
use risk::{RiskLevel, classify};
use serde_json::json;
use std::env;
use std::path::PathBuf;
use store::{RunStatus, RunStore};
use uuid::Uuid;

struct MergeRun {
    mode: String,
    target: String,
    args: Vec<String>,
    apply_ai: bool,
}

struct CommitRun {
    plan_only: bool,
    intent: Option<String>,
    args: Vec<String>,
    resolved_config: config::ResolvedConfig,
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    let cwd = env::current_dir().context("failed to read current directory")?;
    dispatch(cli, cwd).await
}

async fn dispatch(cli: Cli, cwd: PathBuf) -> Result<()> {
    let run_id = Uuid::new_v4();
    let repo = GitExec::discover_repo(&cwd).await?;
    if let Some(repo_ctx) = &repo {
        config::ensure_repo_config(repo_ctx).await?;
    }
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
        CommandKind::Status { args } => {
            run_git_passthrough(
                "status",
                args,
                cwd.clone(),
                repo.clone(),
                store.clone(),
                emitter,
            )
            .await
        }
        CommandKind::Diff { args } => {
            run_git_passthrough(
                "diff",
                args,
                cwd.clone(),
                repo.clone(),
                store.clone(),
                emitter,
            )
            .await
        }
        CommandKind::Add { args } => {
            run_git_passthrough(
                "add",
                args,
                cwd.clone(),
                repo.clone(),
                store.clone(),
                emitter,
            )
            .await
        }
        CommandKind::Commit { plan, intent, args } => {
            run_commit(
                CommitRun {
                    plan_only: plan,
                    intent,
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
        CommandKind::Branch { args } => {
            run_git_passthrough(
                "branch",
                args,
                cwd.clone(),
                repo.clone(),
                store.clone(),
                emitter,
            )
            .await
        }
        CommandKind::Switch { args } => {
            run_git_passthrough(
                "switch",
                args,
                cwd.clone(),
                repo.clone(),
                store.clone(),
                emitter,
            )
            .await
        }
        CommandKind::Stash { args } => {
            run_git_passthrough(
                "stash",
                args,
                cwd.clone(),
                repo.clone(),
                store.clone(),
                emitter,
            )
            .await
        }
        CommandKind::Log { args } => {
            run_git_passthrough(
                "log",
                args,
                cwd.clone(),
                repo.clone(),
                store.clone(),
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
                cwd.clone(),
                repo.clone(),
                store.clone(),
                emitter,
            )
            .await
        }
        CommandKind::Sync { merge } => {
            run_sync(merge, cwd.clone(), repo.clone(), store.clone(), emitter).await
        }
        CommandKind::Ask { prompt } => {
            run_ask(prompt.join(" "), repo.as_ref(), store.clone(), emitter).await
        }
        CommandKind::Rollback { run_id } => {
            run_rollback(run_id, cwd.clone(), repo.clone(), store.clone(), emitter).await
        }
        CommandKind::Runs => run_runs(cwd.clone(), repo.clone(), emitter).await,
        CommandKind::Trace { run_id } => {
            run_trace(run_id, cwd.clone(), repo.clone(), emitter).await
        }
        CommandKind::Doctor => run_doctor(cwd.clone(), repo.clone(), emitter).await,
        CommandKind::Config { command } => run_config(command, repo.as_ref(), emitter).await,
        CommandKind::Scopes { command } => {
            run_scopes(command, cwd.clone(), repo.clone(), store.clone(), emitter).await
        }
        CommandKind::External(args) => {
            run_external(args, cwd.clone(), repo.clone(), store.clone(), emitter).await
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
        })
        .await;
    }
    result
}

async fn run_git_passthrough(
    subcommand: &str,
    args: Vec<String>,
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    store: Option<RunStore>,
    emitter: &mut Emitter,
) -> Result<()> {
    let git = GitExec::new(cwd, repo.clone());
    if let (Some(store), true) = (&store, matches!(subcommand, "merge" | "rebase" | "sync")) {
        let backup_ref = git.create_backup_ref(store.run_id()).await?;
        store.set_backup_ref(Some(backup_ref))?;
    }
    emitter
        .emit(
            "phase_changed",
            Some("exec"),
            Some(format!("running git {subcommand}")),
            Some(json!({
                "git_args": std::iter::once(subcommand.to_string())
                    .chain(args.iter().cloned())
                    .collect::<Vec<_>>()
            })),
        )
        .await?;

    let git_args = std::iter::once(subcommand.to_string())
        .chain(args)
        .collect::<Vec<_>>();
    let outcome = git.run(&git_args, emitter).await?;
    if !outcome.success {
        return Err(anyhow!("git {} failed", subcommand));
    }
    emitter
        .emit(
            "verify_finished",
            Some("verify"),
            Some("git command exited cleanly".to_string()),
            Some(json!({ "success": true })),
        )
        .await?;
    Ok(())
}

async fn run_merge_like(
    request: MergeRun,
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    store: Option<RunStore>,
    emitter: &mut Emitter,
) -> Result<()> {
    let MergeRun {
        mode,
        target,
        args,
        apply_ai,
    } = request;
    let repo_ctx = repo
        .clone()
        .ok_or_else(|| anyhow!("{mode} requires a git repository"))?;
    let git = GitExec::new(cwd, Some(repo_ctx));
    if let Some(store) = &store {
        let backup_ref = git.create_backup_ref(store.run_id()).await?;
        store.set_backup_ref(Some(backup_ref))?;
    }

    let mut git_args = vec![mode.clone(), target];
    git_args.extend(args);
    emitter
        .emit(
            "phase_changed",
            Some("exec"),
            Some(format!("running git {mode}")),
            Some(json!({ "git_args": git_args })),
        )
        .await?;

    let outcome = git.run(&git_args, emitter).await?;
    if outcome.success {
        emitter
            .emit(
                "verify_finished",
                Some("verify"),
                Some(format!("{mode} completed without conflicts")),
                Some(json!({ "success": true })),
            )
            .await?;
        return Ok(());
    }

    let conflicts = git.unresolved_conflicts().await?;
    if conflicts.is_empty() {
        return Err(anyhow!("git {mode} failed"));
    }
    if let Some(store) = &store {
        store.set_conflicts(conflicts.clone())?;
    }
    emitter
        .emit(
            "conflict_detected",
            Some("exec"),
            Some(format!("{mode} produced conflicts")),
            Some(json!({ "files": conflicts })),
        )
        .await?;

    if apply_ai {
        let patch =
            attempt_ai_resolution(&git, repo.as_ref(), &conflicts, store.as_ref(), emitter).await?;
        maybe_apply_patch(&git, patch, store, emitter).await?;
    }
    Err(anyhow!("{mode} stopped on conflicts"))
}

async fn run_sync(
    merge: bool,
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    store: Option<RunStore>,
    emitter: &mut Emitter,
) -> Result<()> {
    let repo_ctx = repo.ok_or_else(|| anyhow!("sync requires a git repository"))?;
    let git = GitExec::new(cwd, Some(repo_ctx));
    if let Some(store) = &store {
        let backup_ref = git.create_backup_ref(store.run_id()).await?;
        store.set_backup_ref(Some(backup_ref))?;
    }
    emitter
        .emit(
            "phase_changed",
            Some("exec"),
            Some("fetching remote updates".to_string()),
            None,
        )
        .await?;
    let fetch = git
        .run(
            &[
                "fetch".to_string(),
                "--all".to_string(),
                "--prune".to_string(),
            ],
            emitter,
        )
        .await?;
    if !fetch.success {
        return Err(anyhow!("git fetch failed"));
    }

    emitter
        .emit(
            "phase_changed",
            Some("exec"),
            Some("pulling current branch".to_string()),
            Some(json!({ "merge": merge })),
        )
        .await?;
    let pull_args = if merge {
        vec!["pull".to_string(), "--no-rebase".to_string()]
    } else {
        vec!["pull".to_string(), "--rebase".to_string()]
    };
    let pull = git.run(&pull_args, emitter).await?;
    if !pull.success {
        return Err(anyhow!("git pull failed"));
    }
    emitter
        .emit(
            "verify_finished",
            Some("verify"),
            Some("sync completed".to_string()),
            Some(json!({ "success": true })),
        )
        .await?;
    Ok(())
}

async fn run_config(
    command: ConfigCommand,
    repo: Option<&git::RepoContext>,
    emitter: &mut Emitter,
) -> Result<()> {
    match command {
        ConfigCommand::Show { scope } => {
            let scope = map_config_scope(scope);
            let (config, sources) =
                config::show_config(scope, repo.map(|repo| repo.root_dir.as_path()))?;
            if emitter.json_mode() {
                emitter
                    .emit(
                        "tool_result",
                        Some("done"),
                        Some("config_show".to_string()),
                        Some(json!({
                            "scope": scope.as_str(),
                            "config": config,
                            "sources": sources,
                        })),
                    )
                    .await?;
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "scope": scope.as_str(),
                        "config": config,
                        "sources": sources,
                    }))?
                );
            }
            Ok(())
        }
        ConfigCommand::Get { key, scope } => {
            let scope = map_config_scope(scope);
            let key = ConfigKey::parse(&key).ok_or_else(|| anyhow!("unknown config key"))?;
            let (normalized, value, source) =
                config::get_config_value(scope, key, repo.map(|repo| repo.root_dir.as_path()))?;
            if emitter.json_mode() {
                emitter
                    .emit(
                        "tool_result",
                        Some("done"),
                        Some("config_get".to_string()),
                        Some(json!({
                            "scope": scope.as_str(),
                            "key": normalized,
                            "value": value,
                            "source": source,
                        })),
                    )
                    .await?;
            } else {
                println!("{normalized} = {value} ({source})");
            }
            Ok(())
        }
        ConfigCommand::Set { key, value, scope } => {
            let scope = map_config_writable_scope(scope);
            let key = ConfigKey::parse(&key).ok_or_else(|| anyhow!("unknown config key"))?;
            let path = config::set_config_value(
                scope,
                key,
                &value,
                repo.map(|repo| repo.root_dir.as_path()),
            )
            .await?;
            if emitter.json_mode() {
                emitter
                    .emit(
                        "tool_result",
                        Some("done"),
                        Some("config_set".to_string()),
                        Some(json!({
                            "scope": scope.as_str(),
                            "key": key.as_str(),
                            "value": value,
                            "path": path.display().to_string(),
                        })),
                    )
                    .await?;
            } else {
                println!("{} = {} -> {}", key.as_str(), value, path.display());
            }
            Ok(())
        }
    }
}

async fn run_scopes(
    command: ScopesCommand,
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    _store: Option<RunStore>,
    emitter: &mut Emitter,
) -> Result<()> {
    let repo_ctx = repo.ok_or_else(|| anyhow!("scopes requires a git repository"))?;
    let git = GitExec::new(cwd, Some(repo_ctx.clone()));
    match command {
        ScopesCommand::Generate => {
            let existing = config::load_repo_config(&repo_ctx.root_dir)
                .map(|cfg| cfg.commit.scopes)
                .unwrap_or_default();
            let subjects = git.recent_subjects(100).await.unwrap_or_default();
            let scopes = generate_scopes(&repo_ctx.root_dir, &subjects, &existing);
            let mut config_doc = config::load_repo_config(&repo_ctx.root_dir).unwrap_or_default();
            config_doc.commit.scopes = scopes.clone();
            let path = config::repo_config_file(&repo_ctx.root_dir);
            std::fs::write(&path, toml::to_string_pretty(&config_doc)?)?;
            emitter
                .emit(
                    "tool_result",
                    Some("done"),
                    Some("scopes_generate".to_string()),
                    Some(json!({ "count": scopes.len(), "scopes": scopes })),
                )
                .await?;
            Ok(())
        }
        ScopesCommand::List => {
            let scopes = list_scopes(&config::load_repo_config(&repo_ctx.root_dir)?.commit.scopes);
            emitter
                .emit(
                    "tool_result",
                    Some("done"),
                    Some("scopes_list".to_string()),
                    Some(json!({ "scopes": scopes })),
                )
                .await?;
            Ok(())
        }
    }
}

async fn run_commit(
    request: CommitRun,
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    store: Option<RunStore>,
    emitter: &mut Emitter,
) -> Result<()> {
    let CommitRun {
        plan_only,
        intent,
        args: _args,
        resolved_config,
    } = request;
    let repo_ctx = repo.ok_or_else(|| anyhow!("commit requires a git repository"))?;
    let git = GitExec::new(cwd.clone(), Some(repo_ctx.clone()));
    let snapshot = git.inspect_snapshot().await?;
    let plan = build_plan(
        &repo_ctx.root_dir,
        &snapshot,
        &resolved_config,
        intent.as_deref(),
    )?;

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

    if plan_only {
        return Ok(());
    }
    if !plan.auto_executable {
        return Err(anyhow!(
            "planner confidence below auto-commit threshold; use --plan to inspect"
        ));
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
        })
        .await;
    }

    if let Some(store) = &store {
        store.finish(RunStatus::Succeeded, None, Some(true))?;
    }
    Ok(())
}

fn map_config_scope(scope: ConfigScopeArg) -> ConfigScope {
    match scope {
        ConfigScopeArg::User => ConfigScope::User,
        ConfigScopeArg::Repo => ConfigScope::Repo,
        ConfigScopeArg::Resolved => ConfigScope::Resolved,
    }
}

fn map_config_writable_scope(scope: ConfigWritableScopeArg) -> ConfigScope {
    match scope {
        ConfigWritableScopeArg::User => ConfigScope::User,
        ConfigWritableScopeArg::Repo => ConfigScope::Repo,
    }
}

async fn run_ask(
    prompt: String,
    repo: Option<&git::RepoContext>,
    store: Option<RunStore>,
    emitter: &mut Emitter,
) -> Result<()> {
    if prompt.trim().is_empty() {
        return Err(anyhow!("ask requires a prompt"));
    }
    let client = AiClient::from_repo(repo)?;
    emitter
        .emit(
            "phase_changed",
            Some("ai_wait"),
            Some("sending ask prompt".to_string()),
            None,
        )
        .await?;
    let reply = client.ask(&prompt, emitter, store.as_ref()).await?;
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

async fn run_rollback(
    run_id: String,
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    _store: Option<RunStore>,
    emitter: &mut Emitter,
) -> Result<()> {
    let repo_ctx = repo.ok_or_else(|| anyhow!("rollback requires a git repository"))?;
    let git = GitExec::new(cwd, Some(repo_ctx.clone()));
    let previous = RunStore::load(repo_ctx.git_dir, &run_id)?;
    let backup_ref = previous
        .backup_ref
        .clone()
        .ok_or_else(|| anyhow!("run {run_id} has no backup ref"))?;

    emitter
        .emit(
            "phase_changed",
            Some("exec"),
            Some(format!("resetting working tree to {backup_ref}")),
            None,
        )
        .await?;
    let reset = git
        .run(
            &[
                "reset".to_string(),
                "--hard".to_string(),
                backup_ref.clone(),
            ],
            emitter,
        )
        .await?;
    if !reset.success {
        return Err(anyhow!("git reset --hard failed"));
    }
    emitter
        .emit(
            "verify_finished",
            Some("verify"),
            Some("rollback completed".to_string()),
            Some(json!({ "success": true, "backup_ref": backup_ref })),
        )
        .await?;
    Ok(())
}

async fn run_runs(
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    emitter: &mut Emitter,
) -> Result<()> {
    let repo_ctx = repo.ok_or_else(|| anyhow!("runs requires a git repository"))?;
    let runs = RunStore::list(repo_ctx.git_dir)?;
    emitter
        .emit(
            "phase_changed",
            Some("sense"),
            Some("listing saved runs".to_string()),
            Some(json!({ "count": runs.len() })),
        )
        .await?;
    if emitter.json_mode() {
        emitter
            .emit(
                "tool_result",
                Some("done"),
                Some("runs".to_string()),
                Some(json!({ "runs": runs })),
            )
            .await?;
    } else {
        for run in runs {
            println!("{}\t{}\t{}", run.run_id, run.command, run.status.as_str());
        }
    }
    let _ = cwd;
    Ok(())
}

async fn run_trace(
    run_id: Option<String>,
    _cwd: PathBuf,
    repo: Option<git::RepoContext>,
    emitter: &mut Emitter,
) -> Result<()> {
    let repo_ctx = repo.ok_or_else(|| anyhow!("trace requires a git repository"))?;
    let run = if let Some(run_id) = run_id {
        RunStore::load(repo_ctx.git_dir.clone(), &run_id)?
    } else {
        RunStore::list(repo_ctx.git_dir.clone())?
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("no runs found"))?
    };
    let events = RunStore::read_events(repo_ctx.git_dir, &run.run_id.to_string())?;
    if emitter.json_mode() {
        emitter
            .emit(
                "tool_result",
                Some("done"),
                Some("trace".to_string()),
                Some(json!({ "run": run, "events": events })),
            )
            .await?;
    } else {
        println!("{}", serde_json::to_string_pretty(&run)?);
        for line in events {
            println!("{line}");
        }
    }
    Ok(())
}

async fn run_doctor(
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    emitter: &mut Emitter,
) -> Result<()> {
    let git_available = GitExec::git_available().await;
    let provider = AiClient::config_from_repo(repo.as_ref()).ok();
    let provider_configured = provider
        .as_ref()
        .is_some_and(|cfg| !cfg.base_url.trim().is_empty() && env::var(&cfg.api_key_env).is_ok());
    let report = json!({
        "cwd": cwd.display().to_string(),
        "git_available": git_available,
        "inside_repo": repo.is_some(),
        "provider_configured": provider_configured,
        "event_stream": true,
        "provider_model": provider.as_ref().map(|cfg| cfg.model.clone()),
        "commit_format": provider.as_ref().map(|cfg| cfg.commit_format.clone()),
        "commit_examples_file": provider.as_ref().map(|cfg| cfg.commit_examples_file.clone()),
    });
    emitter
        .emit(
            "phase_changed",
            Some("sense"),
            Some("collecting environment status".to_string()),
            Some(report.clone()),
        )
        .await?;
    if emitter.json_mode() {
        emitter
            .emit(
                "tool_result",
                Some("done"),
                Some("doctor".to_string()),
                Some(report),
            )
            .await?;
    } else {
        println!("{}", serde_json::to_string_pretty(&report)?);
    }
    Ok(())
}

async fn run_external(
    args: Vec<String>,
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    store: Option<RunStore>,
    emitter: &mut Emitter,
) -> Result<()> {
    if args.is_empty() {
        return Err(anyhow!("no external git command provided"));
    }
    run_git_passthrough(&args[0], args[1..].to_vec(), cwd, repo, store, emitter).await
}

async fn attempt_ai_resolution(
    git: &GitExec,
    repo: Option<&git::RepoContext>,
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

    let patch = client
        .resolve_conflicts(git, conflicts, emitter, store)
        .await?;
    if let Some(store) = store {
        store.write_json("patch.json", &patch)?;
    }
    emitter
        .emit(
            "patch_ready",
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

async fn maybe_apply_patch(
    git: &GitExec,
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

    for file in &patch.files {
        git.write_file(&file.path, &file.resolved_content).await?;
        git.run(&["add".to_string(), file.path.clone()], emitter)
            .await?;
    }

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
