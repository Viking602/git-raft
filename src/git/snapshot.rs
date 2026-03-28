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
