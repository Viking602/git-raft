use crate::commit;
use anyhow::Result;
use serde_json::json;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

pub(super) fn compute_commit_change_set_fingerprint(
    root_dir: &Path,
    planning_inputs: &commit::CommitPlanningInputs,
) -> Result<String> {
    let file_states = planning_inputs
        .changed_files
        .iter()
        .map(|path| {
            let full_path = root_dir.join(path);
            let state = if full_path.exists() {
                let bytes = fs::read(&full_path)?;
                json!({
                    "path": path,
                    "state": "present",
                    "content_hash": hash_bytes(&bytes),
                })
            } else {
                json!({
                    "path": path,
                    "state": "deleted",
                })
            };
            Ok(state)
        })
        .collect::<Result<Vec<_>>>()?;
    let material = serde_json::to_vec(&json!({
        "changed_files": planning_inputs.changed_files,
        "staged_files": planning_inputs.staged_files,
        "unstaged_files": planning_inputs.unstaged_files,
        "untracked_files": planning_inputs.untracked_files,
        "file_states": file_states,
    }))?;
    Ok(hash_bytes(&material))
}

pub(super) fn load_cached_commit_plan(
    git_dir: &Path,
    cache_key: &str,
) -> Result<Option<commit::CommitPlan>> {
    let path = commit_plan_cache_path(git_dir, cache_key);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

pub(super) fn store_cached_commit_plan(
    git_dir: &Path,
    cache_key: &str,
    plan: &commit::CommitPlan,
) -> Result<()> {
    let dir = commit_plan_cache_dir(git_dir);
    fs::create_dir_all(&dir)?;
    fs::write(
        commit_plan_cache_path(git_dir, cache_key),
        serde_json::to_vec_pretty(plan)?,
    )?;
    Ok(())
}

fn commit_plan_cache_dir(git_dir: &Path) -> PathBuf {
    git_dir.join("git-raft").join("commit-plan-cache")
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn commit_plan_cache_path(git_dir: &Path, cache_key: &str) -> PathBuf {
    commit_plan_cache_dir(git_dir).join(format!("{cache_key}.json"))
}
