use crate::git::RepoContext;
use anyhow::{Context, Result};
use serde::Serialize;
use std::env;

#[derive(Debug, Clone, Serialize)]
pub struct AiConfig {
    pub base_url: String,
    pub model: String,
    #[serde(skip_serializing)]
    pub api_key: String,
    pub api_key_env: String,
    pub commit_format: String,
    pub commit_language: String,
    pub commit_use_gitmoji: bool,
    pub commit_include_body: bool,
    pub commit_include_footer: bool,
    pub commit_ignore_paths: Vec<String>,
    pub commit_examples_file: String,
    #[serde(skip_serializing)]
    pub commit_examples: String,
}

pub(super) fn config_from_repo(repo: Option<&RepoContext>) -> Result<AiConfig> {
    let (repo_config, _) = crate::config::resolve_config(repo.map(|repo| repo.root_dir.as_path()))?;

    let base_url = env::var("GIT_RAFT_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| non_empty(&repo_config.provider.base_url))
        .unwrap_or_default();

    let api_key_env = non_empty(&repo_config.provider.api_key_env)
        .unwrap_or_else(|| "GIT_RAFT_API_KEY".to_string());
    let api_key = non_empty(&repo_config.provider.api_key).unwrap_or_default();

    let model = env::var("GIT_RAFT_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| non_empty(&repo_config.provider.model))
        .unwrap_or_else(|| "gpt-4.1-mini".to_string());

    let commit_format =
        non_empty(&repo_config.commit.format).unwrap_or_else(|| "conventional".to_string());
    let commit_language =
        non_empty(&repo_config.commit.language).unwrap_or_else(|| "en".to_string());
    let commit_use_gitmoji = repo_config.commit.use_gitmoji;
    let commit_include_body = repo_config.commit.include_body;
    let commit_include_footer = repo_config.commit.include_footer;
    let commit_ignore_paths = repo_config.commit.ignore_paths.clone();
    let commit_examples_file = non_empty(&repo_config.commit.examples_file)
        .unwrap_or_else(|| ".config/git-raft/commit_examples.md".to_string());
    let commit_examples = repo
        .and_then(|repo| std::fs::read_to_string(repo.root_dir.join(&commit_examples_file)).ok())
        .unwrap_or_default();

    Ok(AiConfig {
        base_url,
        model,
        api_key,
        api_key_env,
        commit_format,
        commit_language,
        commit_use_gitmoji,
        commit_include_body,
        commit_include_footer,
        commit_ignore_paths,
        commit_examples_file,
        commit_examples,
    })
}

pub(super) fn resolve_api_key(config: &AiConfig) -> Result<String> {
    if !config.api_key.trim().is_empty() {
        return Ok(config.api_key.clone());
    }
    env::var(&config.api_key_env).with_context(|| {
        format!(
            "missing provider.api_key or {} for AI calls",
            config.api_key_env
        )
    })
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}
