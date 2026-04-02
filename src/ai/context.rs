use crate::git::{GitExec, RepoContext};

use super::AiConfig;
use super::diff_summary::summarize_diff_stats;
use super::request::RepoContextPayload;

/// Max bytes of raw diff content per file (for summarization input).
const DIFF_PER_FILE_LIMIT: usize = 4096;
/// Max total bytes of raw diff content (for summarization input).
const DIFF_TOTAL_LIMIT: usize = 32768;

pub(crate) async fn collect_repo_context(
    git: &GitExec,
    repo: &RepoContext,
    config: &AiConfig,
) -> RepoContextPayload {
    let git_snapshot = git.inspect_snapshot().await.unwrap_or_default();
    let mut diff_stats = git_snapshot.diff_stats.clone();
    let untracked = git_snapshot.untracked_files.clone();

    // Collect raw diff content for summarization
    let _ = git
        .collect_diff_contents(
            &mut diff_stats,
            &untracked,
            DIFF_PER_FILE_LIMIT,
            DIFF_TOTAL_LIMIT,
        )
        .await;

    // Read untracked file previews for summarization
    let mut untracked_previews = Vec::new();
    for path in &untracked {
        let full_path = repo.root_dir.join(path);
        if let Ok(content) = tokio::fs::read_to_string(&full_path).await {
            if !content.is_empty() {
                untracked_previews.push((path.clone(), content));
            }
        }
    }

    // Build compact structured summary from diffs
    let change_summary = summarize_diff_stats(&diff_stats, &untracked_previews);

    // Strip raw diff_content from stats to keep payload small
    let clean_stats: Vec<_> = diff_stats
        .iter()
        .map(|s| crate::git::DiffStat {
            path: s.path.clone(),
            additions: s.additions,
            deletions: s.deletions,
            diff_content: None,
        })
        .collect();

    RepoContextPayload {
        cwd: repo.root_dir.display().to_string(),
        branch: git_snapshot.branch.clone(),
        diff_stats: clean_stats,
        change_summary,
        recent_subjects: git.recent_subjects(20).await.unwrap_or_default(),
        git_snapshot,
        commit_format: config.commit_format.clone(),
        commit_language: config.commit_language.clone(),
        commit_use_gitmoji: config.commit_use_gitmoji,
        commit_include_body: config.commit_include_body,
        commit_include_footer: config.commit_include_footer,
        commit_ignore_paths: config.commit_ignore_paths.clone(),
        commit_examples_file: config.commit_examples_file.clone(),
        commit_examples: config.commit_examples.clone(),
    }
}
