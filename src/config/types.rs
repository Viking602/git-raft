use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RepoConfig {
    pub provider: ProviderConfig,
    pub commit: CommitConfig,
    pub merge: MergeConfig,
    pub hooks: HooksConfig,
    pub runs: RunsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub api_key_env: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CommitConfig {
    pub format: String,
    pub use_gitmoji: bool,
    pub language: String,
    pub include_body: bool,
    pub include_footer: bool,
    pub examples_file: String,
    pub ignore_paths: Vec<String>,
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
pub struct MergeConfig {
    pub repair_attempts: usize,
    pub verification: Vec<VerificationCommandConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VerificationCommandConfig {
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
    pub merge: MergeConfig,
    pub hooks: HooksConfig,
    pub runs: RunsConfig,
}

pub type ConfigSourceMap = BTreeMap<String, String>;
