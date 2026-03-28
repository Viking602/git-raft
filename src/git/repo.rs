use super::{GitExec, RepoContext};
use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use tokio::process::Command;

impl GitExec {
    pub fn new(cwd: PathBuf, repo: Option<RepoContext>) -> Self {
        Self { cwd, _repo: repo }
    }

    pub async fn discover_repo(cwd: &PathBuf) -> Result<Option<RepoContext>> {
        let git_dir_output = Command::new("git")
            .arg("rev-parse")
            .arg("--git-dir")
            .current_dir(cwd)
            .output()
            .await
            .context("failed to run git rev-parse")?;
        if !git_dir_output.status.success() {
            return Ok(None);
        }
        let git_dir = String::from_utf8(git_dir_output.stdout)?.trim().to_string();
        let path = if PathBuf::from(&git_dir).is_absolute() {
            PathBuf::from(git_dir)
        } else {
            cwd.join(git_dir)
        };
        let root_output = Command::new("git")
            .arg("rev-parse")
            .arg("--show-toplevel")
            .current_dir(cwd)
            .output()
            .await
            .context("failed to resolve repository root")?;
        if !root_output.status.success() {
            return Ok(None);
        }
        let root_dir = PathBuf::from(String::from_utf8(root_output.stdout)?.trim());
        Ok(Some(RepoContext {
            git_dir: path,
            root_dir,
        }))
    }

    pub async fn create_backup_ref(&self, run_id: uuid::Uuid) -> Result<String> {
        let backup_ref = format!("refs/git-raft/backups/{run_id}");
        let output = Command::new("git")
            .args(["update-ref", &backup_ref, "HEAD"])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("failed to create backup ref"));
        }
        Ok(backup_ref)
    }
}
