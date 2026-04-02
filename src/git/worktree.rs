use super::GitExec;
use anyhow::{Result, anyhow};
use tokio::process::Command;

impl GitExec {
    async fn config_value(&self, scope: &str, key: &str) -> Result<Option<String>> {
        let output = Command::new("git")
            .args(["config", scope, "--get", key])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if output.status.success() {
            return Ok(Some(String::from_utf8(output.stdout)?.trim().to_string()));
        }
        if output.status.code() == Some(1) {
            return Ok(None);
        }
        Err(anyhow!("failed to read {scope} {key}"))
    }

    pub async fn resolve_commit(&self, target: &str) -> Result<String> {
        let spec = format!("{target}^{{commit}}");
        let output = Command::new("git")
            .args(["rev-parse", "--verify", &spec])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("failed to resolve commit {target}"));
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    pub async fn unresolved_conflicts(&self) -> Result<Vec<String>> {
        let output = Command::new("git")
            .args(["diff", "--name-only", "--diff-filter=U"])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("failed to inspect unresolved conflicts"));
        }
        let files = String::from_utf8(output.stdout)?
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string)
            .collect();
        Ok(files)
    }

    pub async fn read_stage_file(&self, stage: u8, path: &str) -> Result<String> {
        let spec = format!(":{stage}:{path}");
        let output = Command::new("git")
            .args(["show", &spec])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("failed to read stage {stage} for {path}"));
        }
        Ok(String::from_utf8(output.stdout)?)
    }

    pub async fn read_worktree_file(&self, path: &str) -> Result<String> {
        let full = self.cwd.join(path);
        tokio::fs::read_to_string(full).await.map_err(Into::into)
    }

    pub async fn write_file(&self, path: &str, content: &str) -> Result<()> {
        let full = self.cwd.join(path);
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(full, content).await?;
        Ok(())
    }

    pub async fn recent_subjects(&self, limit: usize) -> Result<Vec<String>> {
        let log = self
            .capture(["log", "--pretty=%s", "-n", &limit.to_string()])
            .await?;
        Ok(log
            .stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect())
    }
    pub async fn stage_files(&self, files: &[String]) -> Result<()> {
        if files.is_empty() {
            return Ok(());
        }
        let mut args = vec!["add".to_string(), "--".to_string()];
        args.extend(files.iter().cloned());
        let output = Command::new("git")
            .args(&args)
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("git add failed"));
        }
        Ok(())
    }

    pub async fn create_commit(&self, message: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("git commit failed"));
        }
        Ok(())
    }

    pub async fn preferred_user(&self) -> Result<Option<(String, String)>> {
        let local_name = self.config_value("--local", "user.name").await?;
        let local_email = self.config_value("--local", "user.email").await?;
        let global_name = self.config_value("--global", "user.name").await?;
        let global_email = self.config_value("--global", "user.email").await?;
        match (local_name.or(global_name), local_email.or(global_email)) {
            (Some(name), Some(email)) => Ok(Some((name, email))),
            _ => Ok(None),
        }
    }

    pub async fn set_local_user(&self, name: &str, email: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["config", "--local", "user.name", name])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("failed to set local user.name"));
        }
        let output = Command::new("git")
            .args(["config", "--local", "user.email", email])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("failed to set local user.email"));
        }
        Ok(())
    }

    pub async fn log_authors(&self, limit: usize) -> Result<Vec<(String, String, String)>> {
        let output = Command::new("git")
            .args(["log", "--pretty=%H%n%an%n%ae", "-n", &limit.to_string()])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("failed to read commit log"));
        }
        let text = String::from_utf8(output.stdout)?;
        let lines: Vec<&str> = text.lines().collect();
        let mut result = Vec::new();
        for chunk in lines.chunks(3) {
            if chunk.len() == 3 {
                result.push((
                    chunk[0].trim().to_string(),
                    chunk[1].trim().to_string(),
                    chunk[2].trim().to_string(),
                ));
            }
        }
        Ok(result)
    }

    pub async fn is_commit_pushed(&self, hash: &str) -> Result<bool> {
        let output = Command::new("git")
            .args(["branch", "-r", "--contains", hash])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Ok(false);
        }
        let text = String::from_utf8(output.stdout)?;
        Ok(!text.trim().is_empty())
    }

    pub async fn rewrite_author(
        &self,
        count: usize,
        name: &str,
        email: &str,
        use_root: bool,
    ) -> Result<()> {
        let exec_cmd = "git commit --amend --reset-author --no-edit";
        let base = if use_root {
            "--root".to_string()
        } else {
            format!("HEAD~{count}")
        };
        let output = Command::new("git")
            .args(["rebase", &base, "--exec", &exec_cmd])
            .env("GIT_SEQUENCE_EDITOR", "true")
            .env("GIT_AUTHOR_NAME", name)
            .env("GIT_AUTHOR_EMAIL", email)
            .env("GIT_COMMITTER_NAME", name)
            .env("GIT_COMMITTER_EMAIL", email)
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let _ = Command::new("git")
                .args(["rebase", "--abort"])
                .current_dir(&self.cwd)
                .output()
                .await;
            return Err(anyhow!("author rewrite failed: {stderr}"));
        }
        Ok(())
    }

    pub async fn force_push_with_lease(&self) -> Result<()> {
        let output = Command::new("git")
            .args(["push", "--force-with-lease"])
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("force push failed: {stderr}"));
        }
        Ok(())
    }
}
