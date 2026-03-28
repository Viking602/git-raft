use super::merge::{default_source_map, merge_config};
use super::types::{ConfigSourceMap, RepoConfig, ResolvedConfig};
use anyhow::{Context, Result};
use std::env;
use std::path::{Path, PathBuf};

pub fn load_user_config() -> Result<Option<RepoConfig>> {
    let path = user_config_file()?;
    if !path.exists() {
        return Ok(None);
    }
    Ok(Some(load_config_file(&path)?))
}

pub fn resolve_config(repo_root: Option<&Path>) -> Result<(ResolvedConfig, ConfigSourceMap)> {
    let mut resolved = RepoConfig::default();
    let mut sources = default_source_map();

    if let Some(user) = load_user_config()? {
        merge_config(&mut resolved, &user, "user", &mut sources);
    }
    if let Some(root) = repo_root {
        let repo_path = repo_config_file(root);
        if repo_path.exists() {
            let repo = load_config_file(&repo_path)?;
            merge_config(&mut resolved, &repo, "repo", &mut sources);
        }
    }

    Ok((
        ResolvedConfig {
            provider: resolved.provider,
            commit: resolved.commit,
            merge: resolved.merge,
            hooks: resolved.hooks,
            runs: resolved.runs,
        },
        sources,
    ))
}

pub fn repo_config_dir(root_dir: &Path) -> PathBuf {
    root_dir.join(".config").join("git-raft")
}

pub fn repo_config_file(root_dir: &Path) -> PathBuf {
    repo_config_dir(root_dir).join("config.toml")
}

pub fn user_config_file() -> Result<PathBuf> {
    Ok(user_config_dir()?.join("config.toml"))
}

pub fn user_config_dir() -> Result<PathBuf> {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .context("missing HOME/USERPROFILE for user config path")?;
    Ok(PathBuf::from(home).join(".config").join("git-raft"))
}

fn load_config_file(path: &Path) -> Result<RepoConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let config = toml::from_str::<RepoConfig>(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(config)
}
