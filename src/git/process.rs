use super::{CaptureOutput, DiffStat, GitExec, GitOutcome, StreamKind, StreamLine};
use crate::events::Emitter;
use anyhow::{Context, Result, anyhow};
use serde_json::json;
use std::collections::BTreeMap;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;

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
