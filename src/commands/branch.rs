use crate::events::Emitter;
use crate::git::{self, GitExec};
use anyhow::{Result, anyhow};
use serde_json::json;
use std::path::PathBuf;

pub(crate) struct BranchRun {
    pub(crate) name: String,
    pub(crate) target: String,
}

pub(crate) async fn run_branch(
    request: BranchRun,
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    emitter: &mut Emitter,
) -> Result<()> {
    let BranchRun { name, target } = request;
    let repo_ctx = repo
        .clone()
        .ok_or_else(|| anyhow!("branch requires a git repository"))?;
    let git = GitExec::new(cwd, Some(repo_ctx));

    emitter
        .emit(
            "phase_changed",
            Some("plan"),
            Some(format!("resolving commit {target}")),
            Some(json!({ "branch": name, "target": target })),
        )
        .await?;
    let resolved = git.resolve_commit(&target).await?;

    let git_args = vec![
        "switch".to_string(),
        "-c".to_string(),
        name.clone(),
        resolved.clone(),
    ];
    emitter
        .emit(
            "phase_changed",
            Some("exec"),
            Some(format!("creating branch {name} from {resolved}")),
            Some(json!({ "git_args": git_args })),
        )
        .await?;

    let outcome = git.run(&git_args, emitter).await?;
    if !outcome.success {
        return Err(anyhow!("git switch failed"));
    }

    emitter
        .emit(
            "verify_finished",
            Some("verify"),
            Some(format!("switched to branch {name}")),
            Some(json!({ "branch": name, "commit": resolved })),
        )
        .await?;
    Ok(())
}
