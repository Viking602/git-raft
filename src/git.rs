mod process;
mod repo;
mod snapshot;
mod worktree;

use serde::Serialize;
use std::path::PathBuf;

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

pub(super) enum StreamKind {
    Stdout,
    Stderr,
}

pub(super) struct StreamLine {
    pub(super) kind: StreamKind,
    pub(super) line: String,
}

pub(super) struct CaptureOutput {
    pub(super) stdout: String,
}
