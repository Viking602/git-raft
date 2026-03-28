use super::types::{
    CommitConfig, HookRules, MergeConfig, ProviderConfig, RepoConfig, ResolvedConfig, RunsConfig,
};

impl Default for ResolvedConfig {
    fn default() -> Self {
        let base = RepoConfig::default();
        Self {
            provider: base.provider,
            commit: base.commit,
            merge: base.merge,
            hooks: base.hooks,
            runs: base.runs,
        }
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            model: "gpt-4.1-mini".to_string(),
            api_key: String::new(),
            api_key_env: "GIT_RAFT_API_KEY".to_string(),
        }
    }
}

impl Default for CommitConfig {
    fn default() -> Self {
        Self {
            format: "conventional".to_string(),
            use_gitmoji: false,
            language: "en".to_string(),
            include_body: true,
            include_footer: false,
            examples_file: ".config/git-raft/commit_examples.md".to_string(),
            ignore_paths: Vec::new(),
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

impl Default for MergeConfig {
    fn default() -> Self {
        Self {
            repair_attempts: 1,
            verification: Vec::new(),
        }
    }
}
