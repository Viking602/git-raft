use super::types::{ConfigSourceMap, RepoConfig};

pub(super) fn merge_config(
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
    if !incoming.provider.api_key.is_empty() {
        resolved.provider.api_key = incoming.provider.api_key.clone();
        sources.insert("provider.api_key".to_string(), source.to_string());
    }
    if !incoming.provider.api_key_env.is_empty() {
        resolved.provider.api_key_env = incoming.provider.api_key_env.clone();
        sources.insert("provider.api_key_env".to_string(), source.to_string());
    }

    if !incoming.commit.format.is_empty() {
        resolved.commit.format = incoming.commit.format.clone();
        sources.insert("commit.format".to_string(), source.to_string());
    }
    if incoming.commit.use_gitmoji {
        resolved.commit.use_gitmoji = true;
        sources.insert("commit.use_gitmoji".to_string(), source.to_string());
    }
    if !incoming.commit.language.is_empty() {
        resolved.commit.language = incoming.commit.language.clone();
        sources.insert("commit.language".to_string(), source.to_string());
    }
    resolved.commit.include_body = incoming.commit.include_body;
    sources.insert("commit.include_body".to_string(), source.to_string());
    resolved.commit.include_footer = incoming.commit.include_footer;
    sources.insert("commit.include_footer".to_string(), source.to_string());
    if !incoming.commit.examples_file.is_empty() {
        resolved.commit.examples_file = incoming.commit.examples_file.clone();
        sources.insert("commit.examples_file".to_string(), source.to_string());
    }
    if !incoming.commit.ignore_paths.is_empty() {
        resolved.commit.ignore_paths = incoming.commit.ignore_paths.clone();
        sources.insert("commit.ignore_paths".to_string(), source.to_string());
    }
    if !incoming.commit.scopes.is_empty() {
        resolved.commit.scopes = incoming.commit.scopes.clone();
        sources.insert("commit.scopes".to_string(), source.to_string());
    }

    resolved.merge.repair_attempts = incoming.merge.repair_attempts;
    sources.insert("merge.repair_attempts".to_string(), source.to_string());
    resolved.merge.verification = incoming.merge.verification.clone();
    sources.insert("merge.verification".to_string(), source.to_string());

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

pub(super) fn default_source_map() -> ConfigSourceMap {
    [
        "provider.base_url",
        "provider.model",
        "provider.api_key",
        "provider.api_key_env",
        "commit.format",
        "commit.use_gitmoji",
        "commit.language",
        "commit.include_body",
        "commit.include_footer",
        "commit.examples_file",
        "commit.ignore_paths",
        "commit.scopes",
        "merge.repair_attempts",
        "merge.verification",
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
