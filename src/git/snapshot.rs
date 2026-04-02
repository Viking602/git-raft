use super::{DiffStat, GitExec, GitSnapshot};
use anyhow::Result;
use std::collections::{BTreeMap, BTreeSet};

impl GitExec {
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

    /// Collect per-file diff content for tracked files and file previews for untracked files.
    /// Each file's content is truncated to `per_file_limit` bytes, and the total budget is
    /// `total_limit` bytes. Returns the number of files that were truncated or skipped.
    pub async fn collect_diff_contents(
        &self,
        diff_stats: &mut Vec<DiffStat>,
        untracked_files: &[String],
        per_file_limit: usize,
        total_limit: usize,
    ) -> Result<usize> {
        let mut total_bytes = 0usize;
        let mut truncated_count = 0usize;

        // Collect diffs for tracked files (staged + unstaged)
        for stat in diff_stats.iter_mut() {
            if total_bytes >= total_limit {
                truncated_count += 1;
                continue;
            }
            let output = self
                .capture(["diff", "--unified=3", "--", stat.path.as_str()])
                .await;
            let content = match output {
                Ok(o) if !o.stdout.is_empty() => o.stdout,
                _ => {
                    // Try staged diff
                    match self
                        .capture(["diff", "--cached", "--unified=3", "--", stat.path.as_str()])
                        .await
                    {
                        Ok(o) if !o.stdout.is_empty() => o.stdout,
                        _ => continue,
                    }
                }
            };
            let remaining = total_limit.saturating_sub(total_bytes);
            let limit = per_file_limit.min(remaining);
            if content.len() > limit {
                stat.diff_content = Some(format!(
                    "{}\n... [truncated, {} more bytes]",
                    &content[..limit],
                    content.len() - limit,
                ));
                truncated_count += 1;
            } else {
                stat.diff_content = Some(content.clone());
            }
            total_bytes += stat.diff_content.as_ref().map_or(0, |c| c.len());
        }

        // Collect previews for untracked (new) files
        for path in untracked_files {
            if total_bytes >= total_limit {
                truncated_count += 1;
                continue;
            }
            let full_path = self.cwd.join(path);
            let content = match tokio::fs::read_to_string(&full_path).await {
                Ok(c) => c,
                Err(_) => continue, // skip binary or unreadable files
            };
            if content.is_empty() {
                continue;
            }
            let remaining = total_limit.saturating_sub(total_bytes);
            let limit = per_file_limit.min(remaining);
            let preview = if content.len() > limit {
                truncated_count += 1;
                format!(
                    "+++ new file: {path}\n{}\n... [truncated, {} more bytes]",
                    &content[..limit],
                    content.len() - limit,
                )
            } else {
                format!("+++ new file: {path}\n{content}")
            };
            // Append as a DiffStat entry for untracked files
            diff_stats.push(DiffStat {
                path: path.clone(),
                additions: content.lines().count(),
                deletions: 0,
                diff_content: Some(preview.clone()),
            });
            total_bytes += preview.len();
        }

        Ok(truncated_count)
    }
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
