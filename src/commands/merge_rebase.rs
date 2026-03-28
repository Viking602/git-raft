mod resolver;
pub(crate) mod retention;
mod validation;

use crate::config;
use crate::events::Emitter;
use crate::git::{self, GitExec};
use crate::store::RunStore;
use anyhow::{Result, anyhow};
use serde_json::json;
use std::path::PathBuf;

use resolver::resolve_conflicts_with_ai;

pub(crate) struct MergeRun {
    pub(crate) mode: String,
    pub(crate) target: String,
    pub(crate) args: Vec<String>,
    pub(crate) apply_ai: bool,
}

pub(crate) async fn run_merge_like(
    request: MergeRun,
    resolved_config: config::ResolvedConfig,
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
    let git = GitExec::new(cwd.clone(), Some(repo_ctx));
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
        let applied = resolve_conflicts_with_ai(
            &git,
            &mode,
            &cwd,
            repo.as_ref(),
            &resolved_config,
            &conflicts,
            store.as_ref(),
            emitter,
        )
        .await?;
        if applied {
            return Ok(());
        }
    }
    Err(anyhow!("{mode} stopped on conflicts"))
}
