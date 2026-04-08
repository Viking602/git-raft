use crate::commit::CommitGroup;
use crate::config::ResolvedConfig;

pub(super) fn normalize_group_message(group: CommitGroup, config: &ResolvedConfig) -> CommitGroup {
    let parsed_subject = parse_subject_line(group.commit_message.lines().next().unwrap_or("").trim());
    let body = extract_body(&group.commit_message);
    let commit_message = format_message(
        config,
        group.scope.as_deref().or(parsed_subject.scope.as_deref()),
        &group.files,
        &group.rationale,
        parsed_subject.commit_type.as_deref(),
        parsed_subject.summary.as_deref(),
        body.as_deref(),
    );
    CommitGroup {
        commit_message,
        ..group
    }
}

#[derive(Debug, Default)]
struct ParsedSubject {
    commit_type: Option<String>,
    scope: Option<String>,
    summary: Option<String>,
}

fn parse_subject_line(subject: &str) -> ParsedSubject {
    if subject.is_empty() {
        return ParsedSubject::default();
    }
    if let Some((prefix, summary)) = subject.split_once(": ") {
        if let Some((commit_type, scope)) = parse_subject_prefix(prefix.trim()) {
            return ParsedSubject {
                commit_type: Some(commit_type),
                scope,
                summary: non_empty(summary),
            };
        }
        return ParsedSubject {
            commit_type: None,
            scope: None,
            summary: non_empty(summary),
        };
    }
    if let Some((_, summary)) = subject.split_once(' ') {
        return ParsedSubject {
            commit_type: None,
            scope: None,
            summary: non_empty(summary),
        };
    }
    ParsedSubject {
        commit_type: None,
        scope: None,
        summary: non_empty(subject),
    }
}

fn parse_subject_prefix(prefix: &str) -> Option<(String, Option<String>)> {
    if let Some((commit_type, scope)) = prefix.split_once('(')
        && let Some(scope) = scope.strip_suffix(')')
        && is_commit_type_hint(commit_type)
    {
        return Some((
            commit_type.trim().to_string(),
            non_empty(scope),
        ));
    }
    if is_commit_type_hint(prefix) {
        return Some((prefix.trim().to_string(), None));
    }
    None
}

fn is_commit_type_hint(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch == '-' || ch == '_')
}

/// Extract the body: everything after the first blank line.
fn extract_body(message: &str) -> Option<String> {
    message
        .splitn(2, "\n\n")
        .nth(1)
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty())
}

fn format_message(
    config: &ResolvedConfig,
    scope: Option<&str>,
    files: &[String],
    rationale: &str,
    commit_type_hint: Option<&str>,
    summary_hint: Option<&str>,
    body: Option<&str>,
) -> String {
    let language = normalized_commit_language(&config.commit.language);
    let summary = summary_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .filter(|value| summary_matches_language(value, language))
        .filter(|value| !is_placeholder_summary(value, scope, language))
        .map(|value| value.to_string())
        .unwrap_or_else(|| default_summary(scope, files, language));
    let commit_type = commit_type_hint
        .map(str::trim)
        .filter(|value| is_commit_type_hint(value))
        .map(str::to_string)
        .unwrap_or_else(|| infer_commit_type(summary_hint, files));
    let scope = meaningful_scope(scope);
    let use_gitmoji = config.commit.use_gitmoji || config.commit.format == "gitmoji";
    if use_gitmoji {
        return format_full_message(
            config,
            format!("{} {}", emoji_for_type(&commit_type), summary),
            files,
            rationale,
            body,
        );
    }
    match config.commit.format.as_str() {
        "simple" => capitalize_first(&summary),
        "angular" | "conventional" => {
            let subject = if let Some(scope) = scope.filter(|scope| !scope.is_empty()) {
                format!("{commit_type}({scope}): {summary}")
            } else {
                format!("{commit_type}: {summary}")
            };
            format_full_message(config, subject, files, rationale, body)
        }
        _ => {
            let subject = if let Some(scope) = scope.filter(|scope| !scope.is_empty()) {
                format!("{commit_type}({scope}): {summary}")
            } else {
                format!("{commit_type}: {summary}")
            };
            format_full_message(config, subject, files, rationale, body)
        }
    }
}

fn format_full_message(
    config: &ResolvedConfig,
    subject: String,
    files: &[String],
    _rationale: &str,
    body: Option<&str>,
) -> String {
    let mut message = subject;
    // Include body with change details if present
    if config.commit.include_body {
        if let Some(body_text) = body {
            message.push_str("\n\n");
            message.push_str(body_text);
        }
    }
    if config.commit.include_footer && !files.is_empty() && config.commit.format != "simple" {
        message.push_str("\n\n");
        message.push_str(&build_footer(files));
    }
    message
}

fn build_footer(files: &[String]) -> String {
    format!("Files: {}", files.join(", "))
}

fn default_summary(scope: Option<&str>, files: &[String], language: &str) -> String {
    let scope = meaningful_scope(scope);
    match language {
        "zh" => match scope {
            Some(scope) => format!("更新 {scope} 相关改动"),
            None if files.len() > 1 => "更新多模块改动".to_string(),
            None if !files.is_empty() => format!("更新 {}", files[0]),
            None => "更新改动".to_string(),
        },
        _ => match scope {
            Some(scope) => format!("update {scope} changes"),
            None if files.len() > 1 => "update cross-module changes".to_string(),
            None if !files.is_empty() => format!("update {}", files[0]),
            None => "update changes".to_string(),
        },
    }
}

fn normalized_commit_language(language: &str) -> &str {
    match language.trim().to_ascii_lowercase().as_str() {
        "zh" | "zh-cn" | "zh-hans" | "chinese" | "涓枃" => "zh",
        _ => "en",
    }
}

fn summary_matches_language(summary: &str, language: &str) -> bool {
    match language {
        "zh" => contains_cjk(summary),
        _ => !contains_cjk(summary),
    }
}

fn is_placeholder_summary(summary: &str, scope: Option<&str>, language: &str) -> bool {
    let Some(scope) = scope.filter(|scope| is_generic_scope_name(scope)) else {
        return false;
    };
    normalize_text(summary) == normalize_text(&raw_scope_summary(scope, language))
}

fn raw_scope_summary(scope: &str, language: &str) -> String {
    match language {
        "zh" => format!("更新 {scope} 相关改动"),
        _ => format!("update {scope} changes"),
    }
}

fn meaningful_scope(scope: Option<&str>) -> Option<&str> {
    scope
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .filter(|scope| !is_generic_scope_name(scope))
}

fn is_generic_scope_name(scope: &str) -> bool {
    matches!(
        scope.trim().to_ascii_lowercase().as_str(),
        "all" | "repo" | "repository" | "project" | "workspace" | "root" | "misc" | "general"
    )
}

fn normalize_text(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace() && !matches!(ch, ':' | '：' | '-' | '_' | '(' | ')'))
        .flat_map(char::to_lowercase)
        .collect()
}

fn contains_cjk(summary: &str) -> bool {
    summary.chars().any(is_cjk)
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0x2EBF0..=0x2EE5F
            | 0x3007
    )
}

fn infer_commit_type(intent: Option<&str>, files: &[String]) -> String {
    let lowered_intent = intent.unwrap_or_default().to_ascii_lowercase();
    if lowered_intent.contains("fix") || lowered_intent.contains("bug") {
        return "fix".to_string();
    }
    if lowered_intent.contains("refactor") || lowered_intent.contains("cleanup") {
        return "refactor".to_string();
    }
    if lowered_intent.contains("test") {
        return "test".to_string();
    }
    if files
        .iter()
        .all(|file| file.ends_with(".md") || file.starts_with("docs/"))
    {
        return "docs".to_string();
    }
    "feat".to_string()
}

fn emoji_for_type(commit_type: &str) -> &'static str {
    match commit_type {
        "fix" => ":bug:",
        "docs" => ":memo:",
        "refactor" => ":recycle:",
        "test" => ":white_check_mark:",
        _ => ":sparkles:",
    }
}

fn capitalize_first(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit::CommitGroup;
    use crate::config::ResolvedConfig;

    #[test]
    fn normalize_group_message_omits_generic_all_scope_in_zh_fallback() {
        let mut config = ResolvedConfig::default();
        config.commit.language = "zh".to_string();
        config.commit.include_body = false;

        let group = CommitGroup {
            scope: Some("all".to_string()),
            files: vec![
                "app/search/internal/biz/search.go".to_string(),
                "pkg/cache/cache.go".to_string(),
            ],
            commit_message: "refactor(all): update repository search and cache flow".to_string(),
            rationale: "cross-module changes".to_string(),
        };

        let normalized = normalize_group_message(group, &config);
        assert_eq!(normalized.commit_message, "refactor: 更新多模块改动");
    }
}
