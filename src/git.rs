use crate::events::Emitter;
use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Serialize)]
pub struct RepoContext {
    pub git_dir: PathBuf,
    pub root_dir: PathBuf,
}

pub struct GitExec {
    cwd: PathBuf,
    _repo: Option<RepoContext>,
}

pub struct GitOutcome {
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GitSnapshot {
    pub branch: Option<String>,
    pub staged_files: Vec<String>,
    pub unstaged_files: Vec<String>,
    pub untracked_files: Vec<String>,
    pub diff_stats: Vec<DiffStat>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffStat {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
}

enum StreamKind {
    Stdout,
    Stderr,
}

struct StreamLine {
    kind: StreamKind,
    line: String,
}

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

    pub async fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|status| status.success())
            .unwrap_or(false)
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

    pub async fn diff_check(&self) -> Result<bool> {
        let output = Command::new("git")
            .args(["diff", "--check"])
            .current_dir(&self.cwd)
            .output()
            .await?;
        Ok(output.status.success())
    }

    pub async fn inspect_snapshot(&self) -> Result<GitSnapshot> {
        let status = self
            .capture([
                "status",
                "--porcelain=1",
                "--branch",
                "--untracked-files=all",
            ])
            .await?;
        let mut branch = None;
        let mut staged = BTreeSet::new();
        let mut unstaged = BTreeSet::new();
        let mut untracked = BTreeSet::new();

        for line in status.stdout.lines() {
            if let Some(name) = line.strip_prefix("## ") {
                branch = Some(name.split("...").next().unwrap_or(name).trim().to_string());
                continue;
            }
            if line.len() < 4 {
                continue;
            }
            let bytes = line.as_bytes();
            let x = bytes[0] as char;
            let y = bytes[1] as char;
            let mut path = line[3..].to_string();
            if let Some((_, renamed)) = path.split_once(" -> ") {
                path = renamed.to_string();
            }
            if x == '?' && y == '?' {
                untracked.insert(path);
                continue;
            }
            if x != ' ' {
                staged.insert(path.clone());
            }
            if y != ' ' {
                unstaged.insert(path);
            }
        }

        let mut stats = BTreeMap::<String, DiffStat>::new();
        self.merge_numstat(&mut stats, ["diff", "--numstat"])
            .await?;
        self.merge_numstat(&mut stats, ["diff", "--cached", "--numstat"])
            .await?;

        Ok(GitSnapshot {
            branch,
            staged_files: staged.into_iter().collect(),
            unstaged_files: unstaged.into_iter().collect(),
            untracked_files: untracked.into_iter().collect(),
            diff_stats: stats.into_values().collect(),
        })
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

    pub async fn run(&self, args: &[String], emitter: &mut Emitter) -> Result<GitOutcome> {
        let mut cmd = Command::new("git");
        cmd.args(args)
            .current_dir(&self.cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn git {:?}", args))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("missing child stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("missing child stderr"))?;
        let (tx, mut rx) = mpsc::unbounded_channel();
        spawn_reader(stdout, StreamKind::Stdout, tx.clone());
        spawn_reader(stderr, StreamKind::Stderr, tx);

        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut wait = Box::pin(child.wait());
        let mut status = None;
        let mut last_activity = Instant::now();
        let mut stream_done = false;

        loop {
            tokio::select! {
                maybe_line = rx.recv(), if !stream_done => {
                    match maybe_line {
                        Some(line) => {
                            last_activity = Instant::now();
                            let event_type = match line.kind {
                                StreamKind::Stdout => "git_stdout",
                                StreamKind::Stderr => "git_stderr",
                            };
                            emitter.emit(event_type, Some("exec"), Some(line.line), None).await?;
                        }
                        None => stream_done = true,
                    }
                }
                exit = &mut wait, if status.is_none() => {
                    status = Some(exit?);
                    last_activity = Instant::now();
                    if stream_done {
                        break;
                    }
                }
                _ = interval.tick() => {
                    if last_activity.elapsed() >= Duration::from_secs(1) {
                        emitter.emit(
                            "heartbeat",
                            Some("exec"),
                            Some(format!("waiting for git {}", args.join(" "))),
                            Some(json!({ "git_args": args })),
                        ).await?;
                        last_activity = Instant::now();
                    }
                }
            }
            if status.is_some() && stream_done {
                break;
            }
        }

        let success = status.map(|status| status.success()).unwrap_or(false);
        Ok(GitOutcome { success })
    }

    async fn capture<const N: usize>(&self, args: [&str; N]) -> Result<CaptureOutput> {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.cwd)
            .output()
            .await?;
        if !output.status.success() {
            return Err(anyhow!("git {:?} failed", args));
        }
        Ok(CaptureOutput {
            stdout: String::from_utf8(output.stdout)?,
        })
    }

    async fn merge_numstat<const N: usize>(
        &self,
        stats: &mut BTreeMap<String, DiffStat>,
        args: [&str; N],
    ) -> Result<()> {
        let output = self.capture(args).await?;
        for line in output.stdout.lines() {
            let parts = line.split('\t').collect::<Vec<_>>();
            if parts.len() != 3 {
                continue;
            }
            let additions = parts[0].parse::<usize>().unwrap_or(0);
            let deletions = parts[1].parse::<usize>().unwrap_or(0);
            let path = parts[2].to_string();
            let entry = stats.entry(path.clone()).or_insert(DiffStat {
                path,
                additions: 0,
                deletions: 0,
            });
            entry.additions += additions;
            entry.deletions += deletions;
        }
        Ok(())
    }
}

pub struct CaptureOutput {
    pub stdout: String,
}

impl GitSnapshot {
    pub fn all_changed_files(&self) -> Vec<String> {
        let mut files = BTreeSet::new();
        for file in &self.staged_files {
            files.insert(file.clone());
        }
        for file in &self.unstaged_files {
            files.insert(file.clone());
        }
        for file in &self.untracked_files {
            files.insert(file.clone());
        }
        files.into_iter().collect()
    }
}

fn spawn_reader<T>(stream: T, kind: StreamKind, tx: mpsc::UnboundedSender<StreamLine>)
where
    T: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let _ = tx.send(StreamLine {
                kind: match kind {
                    StreamKind::Stdout => StreamKind::Stdout,
                    StreamKind::Stderr => StreamKind::Stderr,
                },
                line,
            });
        }
    });
}
