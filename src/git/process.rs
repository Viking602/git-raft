use super::{CaptureOutput, DiffStat, GitExec, GitOutcome, GitSnapshot, StreamKind, StreamLine};
use crate::events::Emitter;
use anyhow::{Context, Result, anyhow};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

struct CaptureHeartbeat<'a> {
    emitter: &'a mut Emitter,
    phase: &'a str,
    message: &'a str,
}

impl GitExec {
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

    pub(super) async fn capture<const N: usize>(&self, args: [&str; N]) -> Result<CaptureOutput> {
        self.capture_internal(args, None).await
    }

    pub(super) async fn capture_with_heartbeat<const N: usize>(
        &self,
        args: [&str; N],
        emitter: &mut Emitter,
        phase: &str,
        message: &str,
    ) -> Result<CaptureOutput> {
        self.capture_internal(
            args,
            Some(CaptureHeartbeat {
                emitter,
                phase,
                message,
            }),
        )
        .await
    }

    async fn capture_internal<const N: usize>(
        &self,
        args: [&str; N],
        mut heartbeat: Option<CaptureHeartbeat<'_>>,
    ) -> Result<CaptureOutput> {
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
        let mut captured_stdout = Vec::new();
        let mut captured_stderr = Vec::new();

        loop {
            tokio::select! {
                maybe_line = rx.recv(), if !stream_done => {
                    match maybe_line {
                        Some(line) => {
                            last_activity = Instant::now();
                            match line.kind {
                                StreamKind::Stdout => captured_stdout.push(line.line),
                                StreamKind::Stderr => captured_stderr.push(line.line),
                            }
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
                    if let Some(heartbeat) = heartbeat.as_mut()
                        && last_activity.elapsed() >= Duration::from_secs(1)
                    {
                        heartbeat.emitter.emit(
                            "heartbeat",
                            Some(heartbeat.phase),
                            Some(heartbeat.message.to_string()),
                            Some(json!({ "git_args": args.to_vec() })),
                        ).await?;
                        last_activity = Instant::now();
                    }
                }
            }
            if status.is_some() && stream_done {
                break;
            }
        }

        if !status.map(|status| status.success()).unwrap_or(false) {
            let stderr = captured_stderr.join("\n");
            if stderr.trim().is_empty() {
                return Err(anyhow!("git {:?} failed", args));
            }
            return Err(anyhow!("git {:?} failed: {}", args, stderr.trim()));
        }

        Ok(CaptureOutput {
            stdout: captured_stdout.join("\n"),
        })
    }

    pub(super) async fn merge_numstat<const N: usize>(
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

    pub(super) async fn merge_numstat_with_heartbeat<const N: usize>(
        &self,
        stats: &mut BTreeMap<String, DiffStat>,
        args: [&str; N],
        emitter: &mut Emitter,
        phase: &str,
        message: &str,
    ) -> Result<()> {
        let output = self
            .capture_with_heartbeat(args, emitter, phase, message)
            .await?;
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

    pub async fn inspect_snapshot_with_heartbeat(
        &self,
        emitter: &mut Emitter,
        phase: &str,
        message: &str,
    ) -> Result<GitSnapshot> {
        let status = self
            .capture_with_heartbeat(
                [
                    "status",
                    "--porcelain=1",
                    "--branch",
                    "--untracked-files=all",
                ],
                emitter,
                phase,
                message,
            )
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
        self.merge_numstat_with_heartbeat(
            &mut stats,
            ["diff", "--numstat"],
            emitter,
            phase,
            message,
        )
        .await?;
        self.merge_numstat_with_heartbeat(
            &mut stats,
            ["diff", "--cached", "--numstat"],
            emitter,
            phase,
            message,
        )
        .await?;

        Ok(GitSnapshot {
            branch,
            staged_files: staged.into_iter().collect(),
            unstaged_files: unstaged.into_iter().collect(),
            untracked_files: untracked.into_iter().collect(),
            diff_stats: stats.into_values().collect(),
        })
    }

    #[allow(dead_code)]
    pub async fn recent_subjects_with_heartbeat(
        &self,
        limit: usize,
        emitter: &mut Emitter,
        phase: &str,
        message: &str,
    ) -> Result<Vec<String>> {
        let log = self
            .capture_with_heartbeat(
                ["log", "--pretty=%s", "-n", &limit.to_string()],
                emitter,
                phase,
                message,
            )
            .await?;
        Ok(log
            .stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect())
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
