use crate::git::RepoContext;
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RepoConfig {
    pub provider: ProviderConfig,
    pub commit: CommitConfig,
    pub hooks: HooksConfig,
    pub runs: RunsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CommitConfig {
    pub format: String,
    pub examples_file: String,
    pub scopes: Vec<CommitScope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CommitScope {
    pub name: String,
    pub description: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HooksConfig {
    pub rules: HookRules,
    pub external: Vec<ExternalHookConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HookRules {
    pub validate_message_format: bool,
    pub scope_required: bool,
    pub empty_group: bool,
    pub max_group_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ExternalHookConfig {
    pub event: String,
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RunsConfig {
    pub dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedConfig {
    pub provider: ProviderConfig,
    pub commit: CommitConfig,
    pub hooks: HooksConfig,
    pub runs: RunsConfig,
}

pub type ConfigSourceMap = BTreeMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigScope {
    User,
    Repo,
    Resolved,
}

impl Default for ResolvedConfig {
    fn default() -> Self {
        let base = RepoConfig::default();
        Self {
            provider: base.provider,
            commit: base.commit,
            hooks: base.hooks,
            runs: base.runs,
        }
    }
}

impl ConfigScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Repo => "repo",
            Self::Resolved => "resolved",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ConfigKey {
    ProviderBaseUrl,
    ProviderModel,
    ProviderApiKeyEnv,
    CommitFormat,
    CommitExamplesFile,
    HooksValidateMessageFormat,
    HooksScopeRequired,
    HooksEmptyGroup,
    HooksMaxGroupCount,
    RunsDir,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            model: "gpt-4.1-mini".to_string(),
            api_key_env: "GIT_RAFT_API_KEY".to_string(),
        }
    }
}

impl Default for CommitConfig {
    fn default() -> Self {
        Self {
            format: "conventional".to_string(),
            examples_file: ".config/git-raft/commit_examples.md".to_string(),
            scopes: Vec::new(),
        }
    }
}

impl Default for HookRules {
    fn default() -> Self {
        Self {
            validate_message_format: true,
            scope_required: false,
            empty_group: true,
            max_group_count: 8,
        }
    }
}

impl Default for RunsConfig {
    fn default() -> Self {
        Self {
            dir: ".git/git-raft/runs".to_string(),
        }
    }
}

pub async fn ensure_repo_config(repo: &RepoContext) -> Result<()> {
    let dir = repo_config_dir(&repo.root_dir);
    fs::create_dir_all(&dir).await?;

    let config_path = repo_config_file(&repo.root_dir);
    if !fs::try_exists(&config_path).await? {
        fs::write(&config_path, default_repo_config_toml()).await?;
    }

    let examples_path = commit_examples_path(&repo.root_dir);
    if !fs::try_exists(&examples_path).await? {
        if let Some(parent) = examples_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&examples_path, default_commit_examples()).await?;
    }
    Ok(())
}

pub fn load_repo_config(root_dir: &Path) -> Result<RepoConfig> {
    load_config_file(&repo_config_file(root_dir))
}

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
            hooks: resolved.hooks,
            runs: resolved.runs,
        },
        sources,
    ))
}

pub fn show_config(
    scope: ConfigScope,
    repo_root: Option<&Path>,
) -> Result<(RepoConfig, ConfigSourceMap)> {
    match scope {
        ConfigScope::Resolved => {
            let (resolved, sources) = resolve_config(repo_root)?;
            Ok((
                RepoConfig {
                    provider: resolved.provider,
                    commit: resolved.commit,
                    hooks: resolved.hooks,
                    runs: resolved.runs,
                },
                sources,
            ))
        }
        ConfigScope::Repo => {
            let root = repo_root.ok_or_else(|| anyhow!("repo scope requires a repository"))?;
            let config = if repo_config_file(root).exists() {
                load_repo_config(root)?
            } else {
                RepoConfig::default()
            };
            Ok((config, source_map_for_single_scope("repo")))
        }
        ConfigScope::User => {
            let config = load_user_config()?.unwrap_or_default();
            Ok((config, source_map_for_single_scope("user")))
        }
    }
}

pub fn get_config_value(
    scope: ConfigScope,
    key: ConfigKey,
    repo_root: Option<&Path>,
) -> Result<(String, String, String)> {
    let (config, sources) = show_config(scope, repo_root)?;
    let normalized = key.as_str().to_string();
    let value = key.read_from(&config);
    let source = if scope == ConfigScope::Resolved {
        sources
            .get(&normalized)
            .cloned()
            .unwrap_or_else(|| "default".to_string())
    } else {
        scope.as_str().to_string()
    };
    Ok((normalized, value, source))
}

pub async fn set_config_value(
    scope: ConfigScope,
    key: ConfigKey,
    value: &str,
    repo_root: Option<&Path>,
) -> Result<PathBuf> {
    let path = match scope {
        ConfigScope::User => user_config_file()?,
        ConfigScope::Repo => {
            let root = repo_root.ok_or_else(|| anyhow!("repo scope requires a repository"))?;
            repo_config_file(root)
        }
        ConfigScope::Resolved => return Err(anyhow!("cannot write to resolved config")),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut config = if path.exists() {
        load_config_file(&path)?
    } else {
        RepoConfig::default()
    };
    key.write_to(&mut config, value)?;
    std::fs::write(&path, toml::to_string_pretty(&config)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

pub fn repo_config_dir(root_dir: &Path) -> PathBuf {
    root_dir.join(".config").join("git-raft")
}

pub fn repo_config_file(root_dir: &Path) -> PathBuf {
    repo_config_dir(root_dir).join("config.toml")
}

pub fn user_config_file() -> Result<PathBuf> {
    let home = env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .context("missing HOME/USERPROFILE for user config path")?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("git-raft")
        .join("config.toml"))
}

pub fn commit_examples_path(root_dir: &Path) -> PathBuf {
    repo_config_dir(root_dir).join("commit_examples.md")
}

impl ConfigKey {
    pub fn parse(raw: &str) -> Option<Self> {
        let normalized = raw.trim().replace('-', "_").to_ascii_lowercase();
        match normalized.as_str() {
            "provider.base_url" | "provider_base_url" => Some(Self::ProviderBaseUrl),
            "provider.model" | "provider_model" => Some(Self::ProviderModel),
            "provider.api_key_env" | "provider_api_key_env" | "provider.api_key" => {
                Some(Self::ProviderApiKeyEnv)
            }
            "commit.format" | "commit_format" => Some(Self::CommitFormat),
            "commit.examples_file" | "commit_examples_file" => Some(Self::CommitExamplesFile),
            "hooks.rules.validate_message_format"
            | "hooks_rules_validate_message_format"
            | "validate_message_format" => Some(Self::HooksValidateMessageFormat),
            "hooks.rules.scope_required" | "hooks_rules_scope_required" | "scope_required" => {
                Some(Self::HooksScopeRequired)
            }
            "hooks.rules.empty_group" | "hooks_rules_empty_group" | "empty_group" => {
                Some(Self::HooksEmptyGroup)
            }
            "hooks.rules.max_group_count" | "hooks_rules_max_group_count" | "max_group_count" => {
                Some(Self::HooksMaxGroupCount)
            }
            "runs.dir" | "runs_dir" => Some(Self::RunsDir),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProviderBaseUrl => "provider.base_url",
            Self::ProviderModel => "provider.model",
            Self::ProviderApiKeyEnv => "provider.api_key_env",
            Self::CommitFormat => "commit.format",
            Self::CommitExamplesFile => "commit.examples_file",
            Self::HooksValidateMessageFormat => "hooks.rules.validate_message_format",
            Self::HooksScopeRequired => "hooks.rules.scope_required",
            Self::HooksEmptyGroup => "hooks.rules.empty_group",
            Self::HooksMaxGroupCount => "hooks.rules.max_group_count",
            Self::RunsDir => "runs.dir",
        }
    }

    fn read_from(self, config: &RepoConfig) -> String {
        match self {
            Self::ProviderBaseUrl => config.provider.base_url.clone(),
            Self::ProviderModel => config.provider.model.clone(),
            Self::ProviderApiKeyEnv => config.provider.api_key_env.clone(),
            Self::CommitFormat => config.commit.format.clone(),
            Self::CommitExamplesFile => config.commit.examples_file.clone(),
            Self::HooksValidateMessageFormat => {
                config.hooks.rules.validate_message_format.to_string()
            }
            Self::HooksScopeRequired => config.hooks.rules.scope_required.to_string(),
            Self::HooksEmptyGroup => config.hooks.rules.empty_group.to_string(),
            Self::HooksMaxGroupCount => config.hooks.rules.max_group_count.to_string(),
            Self::RunsDir => config.runs.dir.clone(),
        }
    }

    fn write_to(self, config: &mut RepoConfig, value: &str) -> Result<()> {
        match self {
            Self::ProviderBaseUrl => config.provider.base_url = value.to_string(),
            Self::ProviderModel => config.provider.model = value.to_string(),
            Self::ProviderApiKeyEnv => config.provider.api_key_env = value.to_string(),
            Self::CommitFormat => config.commit.format = value.to_string(),
            Self::CommitExamplesFile => config.commit.examples_file = value.to_string(),
            Self::HooksValidateMessageFormat => {
                config.hooks.rules.validate_message_format = parse_bool(value)?
            }
            Self::HooksScopeRequired => config.hooks.rules.scope_required = parse_bool(value)?,
            Self::HooksEmptyGroup => config.hooks.rules.empty_group = parse_bool(value)?,
            Self::HooksMaxGroupCount => {
                config.hooks.rules.max_group_count = value
                    .parse::<usize>()
                    .with_context(|| format!("invalid usize for {}", self.as_str()))?
            }
            Self::RunsDir => config.runs.dir = value.to_string(),
        }
        Ok(())
    }
}

fn parse_bool(value: &str) -> Result<bool> {
    value
        .parse::<bool>()
        .with_context(|| format!("invalid bool value: {value}"))
}

fn load_config_file(path: &Path) -> Result<RepoConfig> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let config = toml::from_str::<RepoConfig>(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(config)
}

fn merge_config(
    resolved: &mut RepoConfig,
    incoming: &RepoConfig,
    source: &str,
    sources: &mut ConfigSourceMap,
) {
    if !incoming.provider.base_url.is_empty() {
        resolved.provider.base_url = incoming.provider.base_url.clone();
        sources.insert("provider.base_url".to_string(), source.to_string());
    }
    if !incoming.provider.model.is_empty() {
        resolved.provider.model = incoming.provider.model.clone();
        sources.insert("provider.model".to_string(), source.to_string());
    }
    if !incoming.provider.api_key_env.is_empty() {
        resolved.provider.api_key_env = incoming.provider.api_key_env.clone();
        sources.insert("provider.api_key_env".to_string(), source.to_string());
    }

    if !incoming.commit.format.is_empty() {
        resolved.commit.format = incoming.commit.format.clone();
        sources.insert("commit.format".to_string(), source.to_string());
    }
    if !incoming.commit.examples_file.is_empty() {
        resolved.commit.examples_file = incoming.commit.examples_file.clone();
        sources.insert("commit.examples_file".to_string(), source.to_string());
    }
    if !incoming.commit.scopes.is_empty() {
        resolved.commit.scopes = incoming.commit.scopes.clone();
        sources.insert("commit.scopes".to_string(), source.to_string());
    }

    resolved.hooks.rules.validate_message_format = incoming.hooks.rules.validate_message_format;
    sources.insert(
        "hooks.rules.validate_message_format".to_string(),
        source.to_string(),
    );
    resolved.hooks.rules.scope_required = incoming.hooks.rules.scope_required;
    sources.insert("hooks.rules.scope_required".to_string(), source.to_string());
    resolved.hooks.rules.empty_group = incoming.hooks.rules.empty_group;
    sources.insert("hooks.rules.empty_group".to_string(), source.to_string());
    resolved.hooks.rules.max_group_count = incoming.hooks.rules.max_group_count;
    sources.insert(
        "hooks.rules.max_group_count".to_string(),
        source.to_string(),
    );

    if !incoming.hooks.external.is_empty() {
        resolved.hooks.external = incoming.hooks.external.clone();
        sources.insert("hooks.external".to_string(), source.to_string());
    }

    if !incoming.runs.dir.is_empty() {
        resolved.runs.dir = incoming.runs.dir.clone();
        sources.insert("runs.dir".to_string(), source.to_string());
    }
}

fn default_source_map() -> ConfigSourceMap {
    [
        "provider.base_url",
        "provider.model",
        "provider.api_key_env",
        "commit.format",
        "commit.examples_file",
        "commit.scopes",
        "hooks.rules.validate_message_format",
        "hooks.rules.scope_required",
        "hooks.rules.empty_group",
        "hooks.rules.max_group_count",
        "hooks.external",
        "runs.dir",
    ]
    .into_iter()
    .map(|key| (key.to_string(), "default".to_string()))
    .collect()
}

fn source_map_for_single_scope(source: &str) -> ConfigSourceMap {
    default_source_map()
        .into_keys()
        .map(|key| (key, source.to_string()))
        .collect()
}

fn default_repo_config_toml() -> &'static str {
    r#"# git-raft local config
# Choose one commit format preset:
# - conventional: feat(scope): add feature
# - angular: type(scope): summary
# - gitmoji: :sparkles: add feature
# - simple: Add feature

[provider]
base_url = ""
model = "gpt-4.1-mini"
api_key_env = "GIT_RAFT_API_KEY"

[commit]
format = "conventional"
examples_file = ".config/git-raft/commit_examples.md"

[hooks.rules]
validate_message_format = true
scope_required = false
empty_group = true
max_group_count = 8

[runs]
dir = ".git/git-raft/runs"
"#
}

fn default_commit_examples() -> &'static str {
    r#"# git-raft commit format examples

Use this file when AI needs commit message examples.
Keep examples short and realistic.

## conventional
- feat(auth): add OAuth callback handler
- fix(sync): handle detached HEAD during fetch
- docs(config): document local model selection

## angular
- feat(cli): add doctor command output
- fix(git): preserve backup refs on rollback
- docs(agent): update runtime notes

## gitmoji
- :sparkles: add model selection in repo config
- :bug: fix trace persistence after failed merge
- :memo: document commit format presets

## simple
- Add repo-local config generation
- Fix rollback backup lookup
- Document AI commit message presets

## custom
Add your own house style below and set `[commit].format` to a matching label.
"#
}
