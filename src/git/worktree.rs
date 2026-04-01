use super::GitExec;
use anyhow::{Result, anyhow};
use tokio::process::Command;

impl GitExec {
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
}
