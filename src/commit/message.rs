use crate::commit::CommitGroup;
use crate::config::ResolvedConfig;

pub(super) fn normalize_group_message(group: CommitGroup, config: &ResolvedConfig) -> CommitGroup {
    let (summary_hint, body) = extract_summary_and_body(&group.commit_message);
    let commit_message = format_message(
        config,
        group.scope.as_deref(),
        &group.files,
        &group.rationale,
        summary_hint.as_deref(),
        body.as_deref(),
    );
    CommitGroup {
        commit_message,
        ..group
    }
}

/// Extract the subject summary and optional body from a commit message.
/// The body is everything after the first blank line.
fn extract_summary_and_body(message: &str) -> (Option<String>, Option<String>) {
    let subject = message.lines().next().unwrap_or("").trim();
    let summary = if subject.is_empty() {
        None
    } else if let Some((_, s)) = subject.split_once(": ") {
        Some(s.trim().to_string())
    } else if let Some((_, s)) = subject.split_once(' ') {
        Some(s.trim().to_string())
    } else {
        Some(subject.to_string())
    };

    // Extract body: everything after the first blank line
    let body = message
        .splitn(2, "\n\n")
        .nth(1)
        .map(|b| b.trim().to_string())
        .filter(|b| !b.is_empty());

    (summary, body)
}

fn format_message(
    config: &ResolvedConfig,
    scope: Option<&str>,
    files: &[String],
    rationale: &str,
    intent: Option<&str>,
    body: Option<&str>,
) -> String {
    let language = normalized_commit_language(&config.commit.language);
    let summary = intent
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| default_summary(scope, files, language));
    let commit_type = infer_commit_type(intent, files);
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
    match language {
        "zh" => match scope {
            Some(scope) => format!("鏇存柊 {scope} 鐩稿叧鏀瑰姩"),
            None if !files.is_empty() => format!("鏇存柊 {}", files[0]),
            None => "鏇存柊鏀瑰姩".to_string(),
        },
        _ => match scope {
            Some(scope) => format!("update {scope} changes"),
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
