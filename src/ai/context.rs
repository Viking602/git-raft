use crate::git::{GitExec, RepoContext};

use super::{AiConfig, RepoContextPayload};

pub(crate) async fn collect_repo_context(
    git: &GitExec,
    repo: &RepoContext,
    config: &AiConfig,
) -> RepoContextPayload {
    let git_snapshot = git.inspect_snapshot().await.unwrap_or_default();
    RepoContextPayload {
        cwd: repo.root_dir.display().to_string(),
        branch: git_snapshot.branch.clone(),
        diff_stats: git_snapshot.diff_stats.clone(),
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
