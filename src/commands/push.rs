use crate::cli::PushStrategy;
use crate::commands::merge_rebase::{MergeRun, run_merge_like};
use crate::config;
use crate::events::Emitter;
use crate::git::{self, GitExec};
use crate::store::RunStore;
use anyhow::{Result, anyhow};
use serde_json::json;
use std::path::{Path, PathBuf};

pub(crate) struct PushRun {
    pub(crate) remote: String,
    pub(crate) refspec: Option<String>,
    pub(crate) strategy: PushStrategy,
    pub(crate) max_retries: u32,
    pub(crate) force: bool,
}

pub(crate) async fn run_push(
    request: PushRun,
    resolved_config: config::ResolvedConfig,
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    store: Option<RunStore>,
    emitter: &mut Emitter,
) -> Result<()> {
    let repo_ctx = repo
        .clone()
        .ok_or_else(|| anyhow!("push requires a git repository"))?;
    let git = GitExec::new(cwd.clone(), Some(repo_ctx));

    let push_args = build_push_args(&request);

    for attempt in 0..=request.max_retries {
        emitter
            .emit(
                "phase_changed",
                Some("exec"),
                Some(if attempt == 0 {
                    "pushing to remote".to_string()
                } else {
                    format!(
                        "retrying push (attempt {}/{})",
                        attempt, request.max_retries
                    )
                }),
                Some(json!({ "git_args": push_args, "attempt": attempt })),
            )
            .await?;

        let outcome = git.run(&push_args, emitter).await?;
        if outcome.success {
            emitter
                .emit(
                    "push_succeeded",
                    Some("done"),
                    Some("push completed successfully".to_string()),
                    Some(json!({ "attempt": attempt })),
                )
                .await?;
            return Ok(());
        }

        // First attempt failed — try pull then retry
        if attempt < request.max_retries {
            emitter
                .emit(
                    "push_rejected",
                    Some("exec"),
                    Some("push rejected, pulling remote changes".to_string()),
                    Some(json!({ "strategy": format!("{:?}", request.strategy).to_lowercase() })),
                )
                .await?;

            pull_and_resolve(&request, &resolved_config, &cwd, &repo, &store, emitter).await?;
        }
    }

    Err(anyhow!(
        "push failed after {} retries — remote may have diverged further",
        request.max_retries
    ))
}

async fn pull_and_resolve(
    request: &PushRun,
    resolved_config: &config::ResolvedConfig,
    cwd: &Path,
    repo: &Option<git::RepoContext>,
    store: &Option<RunStore>,
    emitter: &mut Emitter,
) -> Result<()> {
    let mode = match request.strategy {
        PushStrategy::Rebase => "rebase",
        PushStrategy::Merge => "merge",
    };

    // Build the target as remote/branch for merge_like
    let git = GitExec::new(cwd.to_path_buf(), repo.clone());
    let branch = git.current_branch().await?;
    let target = format!("{}/{}", request.remote, branch);

    // Fetch first so merge_like has the remote ref
    let fetch_args = vec!["fetch".to_string(), request.remote.clone(), branch.clone()];
    emitter
        .emit(
            "phase_changed",
            Some("exec"),
            Some(format!("fetching from {}", request.remote)),
            Some(json!({ "git_args": fetch_args })),
        )
        .await?;
    let fetch_outcome = git.run(&fetch_args, emitter).await?;
    if !fetch_outcome.success {
        return Err(anyhow!("failed to fetch from {}", request.remote));
    }

    // Use existing merge/rebase with AI conflict resolution
    run_merge_like(
        MergeRun {
            mode: mode.to_string(),
            target,
            args: vec![],
            apply_ai: true,
        },
        resolved_config.clone(),
        cwd.to_path_buf(),
        repo.clone(),
        store.clone(),
        emitter,
    )
    .await
}

fn build_push_args(request: &PushRun) -> Vec<String> {
    let mut args = vec!["push".to_string()];
    if request.force {
        args.push("--force".to_string());
    }
    args.push(request.remote.clone());
    if let Some(refspec) = &request.refspec {
        args.push(refspec.clone());
    }
    args
}
