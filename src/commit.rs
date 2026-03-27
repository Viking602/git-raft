use crate::config::{CommitScope, ResolvedConfig};
use crate::git::GitSnapshot;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const AUTO_COMMIT_MIN_CONFIDENCE: f32 = 0.8;
const DEFAULT_IGNORED_TOOL_DIRS: &[&str] = &[
    ".codex",
    ".claude",
    ".cursor",
    ".windsurf",
    ".zed",
    ".vscode",
    ".idea",
    ".roo",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitPlan {
    pub groups: Vec<CommitGroup>,
    pub confidence: f32,
    pub warnings: Vec<String>,
    pub auto_executable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitGroup {
    pub scope: Option<String>,
    pub files: Vec<String>,
    pub commit_message: String,
    pub rationale: String,
}

#[derive(Debug, Clone)]
pub struct CommitPlanningInputs {
    pub changed_files: Vec<String>,
    pub staged_files: Vec<String>,
    pub unstaged_files: Vec<String>,
    pub untracked_files: Vec<String>,
}

pub fn build_plan(
    root_dir: &Path,
    snapshot: &GitSnapshot,
    config: &ResolvedConfig,
    intent: Option<&str>,
) -> Result<CommitPlan> {
    let planning = collect_planning_inputs(snapshot, config);
    let files = planning.changed_files;
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut exact_matches = 0usize;

    for file in &files {
        let (scope, exact) = infer_scope(file, &config.commit.scopes);
        if exact {
            exact_matches += 1;
        }
        grouped.entry(scope).or_default().push(file.clone());
    }
    grouped = merge_companion_groups(grouped);

    if grouped.is_empty() {
        warnings.push("no file-backed commit groups were generated".to_string());
    }

    let groups = grouped
        .into_iter()
        .map(|(scope, files)| {
            let scope_opt = if scope == "repo" {
                None
            } else {
                Some(scope.clone())
            };
            let rationale = format!("grouped by inferred scope `{scope}`");
            let message = format_message(config, scope_opt.as_deref(), &files, &rationale, intent);
            CommitGroup {
                scope: scope_opt,
                files,
                rationale,
                commit_message: message,
            }
        })
        .collect::<Vec<_>>();

    let mut confidence = if files.is_empty() {
        0.0
    } else {
        0.6 + 0.3 * (exact_matches as f32 / files.len() as f32)
    };
    if groups.len() == 1 {
        confidence += 0.1;
        if files.len() <= 3 {
            confidence = confidence.max(0.85);
        }
    }
    confidence = confidence.min(0.98);
    if groups.len() > 4 {
        warnings.push("planner created many groups; review before committing".to_string());
        confidence = confidence.min(0.74);
    }

    let _ = root_dir;
    Ok(CommitPlan {
        auto_executable: confidence >= AUTO_COMMIT_MIN_CONFIDENCE,
        groups,
        confidence,
        warnings,
    })
}

pub fn collect_planning_inputs(
    snapshot: &GitSnapshot,
    config: &ResolvedConfig,
) -> CommitPlanningInputs {
    let mut changed_files = snapshot
        .all_changed_files()
        .into_iter()
        .filter(|file| !should_ignore_file(file, &config.commit.ignore_paths))
        .collect::<Vec<_>>();
    changed_files.sort();
    changed_files.dedup();

    let filter_files = |files: &[String]| {
        let mut kept = files
            .iter()
            .filter(|file| !should_ignore_file(file, &config.commit.ignore_paths))
            .cloned()
            .collect::<Vec<_>>();
        kept.sort();
        kept.dedup();
        kept
    };

    CommitPlanningInputs {
        changed_files,
        staged_files: filter_files(&snapshot.staged_files),
        unstaged_files: filter_files(&snapshot.unstaged_files),
        untracked_files: filter_files(&snapshot.untracked_files),
    }
}

pub fn generate_scopes(
    root_dir: &Path,
    subjects: &[String],
    existing: &[CommitScope],
) -> Vec<CommitScope> {
    let mut scopes = BTreeMap::<String, CommitScope>::new();

    for scope in existing {
        if !scope.name.trim().is_empty() {
            scopes.insert(scope.name.clone(), scope.clone());
        }
    }

    for scope in discover_from_tree(root_dir) {
        scopes.entry(scope.name.clone()).or_insert(scope);
    }
    for scope in discover_from_history(subjects) {
        scopes
            .entry(scope.name.clone())
            .and_modify(|current| {
                for path in &scope.paths {
                    if !current.paths.contains(path) {
                        current.paths.push(path.clone());
                    }
                }
                if current.description.is_empty() {
                    current.description = scope.description.clone();
                }
            })
            .or_insert(scope);
    }

    scopes.into_values().collect()
}

pub fn list_scopes(scopes: &[CommitScope]) -> Vec<CommitScope> {
    let mut ordered = scopes.to_vec();
    ordered.sort_by(|left, right| left.name.cmp(&right.name));
    ordered
}

fn discover_from_tree(root_dir: &Path) -> Vec<CommitScope> {
    let mut scopes = Vec::new();
    for dir in ["src", "tests", "docs"] {
        let path = root_dir.join(dir);
        if !path.is_dir() {
            continue;
        }
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            let name = if file_type.is_dir() {
                entry.file_name().to_string_lossy().to_string()
            } else {
                entry
                    .path()
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
                    .unwrap_or_default()
            };
            if name.is_empty() {
                continue;
            }
            let path_value = if file_type.is_dir() {
                format!("{dir}/{name}")
            } else {
                format!("{dir}/{}.rs", name)
            };
            scopes.push(CommitScope {
                name: name.clone(),
                description: format!("Changes related to {name}"),
                paths: vec![path_value],
            });
        }
    }

    for entry in fs::read_dir(root_dir).into_iter().flatten().flatten() {
        if !entry
            .file_type()
            .map(|file_type| file_type.is_dir())
            .unwrap_or(false)
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if [".git", ".config", "target", "src", "tests", "docs"].contains(&name.as_str()) {
            continue;
        }
        scopes.push(CommitScope {
            name: name.clone(),
            description: format!("Changes related to {name}"),
            paths: vec![name],
        });
    }
    dedupe_scopes(scopes)
}

fn discover_from_history(subjects: &[String]) -> Vec<CommitScope> {
    let mut scopes = Vec::new();
    for subject in subjects {
        let Some(start) = subject.find('(') else {
            continue;
        };
        let Some(end) = subject[start + 1..].find(')') else {
            continue;
        };
        let scope = &subject[start + 1..start + 1 + end];
        if scope.trim().is_empty() {
            continue;
        }
        scopes.push(CommitScope {
            name: scope.to_string(),
            description: format!("History-derived scope `{scope}`"),
            paths: candidate_paths_for_scope(scope),
        });
    }
    dedupe_scopes(scopes)
}

fn candidate_paths_for_scope(scope: &str) -> Vec<String> {
    vec![
        format!("src/{scope}"),
        format!("src/{scope}.rs"),
        format!("tests/{scope}"),
        format!("docs/{scope}"),
    ]
}

fn dedupe_scopes(scopes: Vec<CommitScope>) -> Vec<CommitScope> {
    let mut map = BTreeMap::<String, CommitScope>::new();
    for mut scope in scopes {
        let mut seen = BTreeSet::new();
        scope.paths.retain(|path| seen.insert(path.clone()));
        map.entry(scope.name.clone())
            .and_modify(|existing| {
                for path in &scope.paths {
                    if !existing.paths.contains(path) {
                        existing.paths.push(path.clone());
                    }
                }
            })
            .or_insert(scope);
    }
    map.into_values().collect()
}

fn infer_scope(file: &str, scopes: &[CommitScope]) -> (String, bool) {
    let normalized = file.replace('\\', "/");
    if is_root_docs_file(&normalized) {
        return ("docs".to_string(), false);
    }
    if is_repo_companion_file(&normalized) {
        return ("repo".to_string(), false);
    }
    let best_match = scopes
        .iter()
        .filter_map(|scope| {
            let matched = scope
                .paths
                .iter()
                .filter(|path| normalized.starts_with(path.as_str()))
                .max_by_key(|path| path.len())?;
            Some((scope.name.clone(), matched.len()))
        })
        .max_by_key(|(_, len)| *len);
    if let Some((scope, _)) = best_match {
        return (scope, true);
    }

    let path = PathBuf::from(&normalized);
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if components.is_empty() {
        return ("repo".to_string(), false);
    }
    if components[0] == "docs" {
        return ("docs".to_string(), false);
    }
    if components[0] == "tests" {
        return ("tests".to_string(), false);
    }
    if components[0] == "src" && components.len() >= 2 {
        let candidate = components[1].trim_end_matches(".rs").to_string();
        return (candidate, false);
    }
    (components[0].clone(), false)
}

fn merge_companion_groups(grouped: BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    let feature_scopes = grouped
        .iter()
        .filter_map(|(scope, files)| {
            if is_feature_scope(scope) && !files.is_empty() {
                Some(scope.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if feature_scopes.len() != 1 {
        return grouped;
    }

    let target_scope = feature_scopes[0].clone();
    let mut merged = BTreeMap::new();
    let mut target_files = Vec::new();

    for (scope, files) in grouped {
        if scope == target_scope {
            target_files.extend(files);
            continue;
        }
        if scope == "repo" && files.iter().all(|file| is_repo_companion_file(file)) {
            target_files.extend(files);
            continue;
        }
        if scope == "docs" && files.iter().all(|file| is_root_docs_file(file)) {
            target_files.extend(files);
            continue;
        }
        merged.insert(scope, files);
    }

    target_files.sort();
    target_files.dedup();
    merged.insert(target_scope, target_files);
    merged
}

fn should_ignore_file(file: &str, custom_rules: &[String]) -> bool {
    let normalized = normalize_rule(file);
    if normalized.starts_with(".config/git-raft/") {
        return true;
    }
    if DEFAULT_IGNORED_TOOL_DIRS
        .iter()
        .any(|rule| matches_ignore_rule(&normalized, rule))
    {
        return true;
    }
    custom_rules
        .iter()
        .any(|rule| matches_ignore_rule(&normalized, rule))
}

fn matches_ignore_rule(path: &str, rule: &str) -> bool {
    let normalized_rule = normalize_rule(rule);
    if normalized_rule.is_empty() {
        return false;
    }
    path == normalized_rule || path.starts_with(&format!("{normalized_rule}/"))
}

fn normalize_rule(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("./")
        .trim_end_matches('/')
        .replace('\\', "/")
}

fn is_feature_scope(scope: &str) -> bool {
    !matches!(scope, "repo" | "docs" | "tests")
}

fn is_root_docs_file(file: &str) -> bool {
    if file.contains('/') {
        return false;
    }
    let lower = file.to_ascii_lowercase();
    lower.ends_with(".md")
        || lower.ends_with(".txt")
        || lower == "license"
        || lower == "license.md"
        || lower == "license.txt"
        || lower.starts_with("readme")
        || lower.starts_with("changelog")
}

fn is_repo_companion_file(file: &str) -> bool {
    let normalized = file.replace('\\', "/");
    if normalized.contains('/') {
        return matches!(normalized.as_str(), ".github/workflows" | ".github/actions");
    }
    matches!(
        normalized.as_str(),
        "Cargo.toml"
            | "Cargo.lock"
            | "rust-toolchain.toml"
            | "package.json"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "bun.lockb"
            | "tsconfig.json"
            | "tsconfig.base.json"
            | "Makefile"
            | "Justfile"
            | "Dockerfile"
            | ".gitignore"
            | ".editorconfig"
    )
}

fn format_message(
    config: &ResolvedConfig,
    scope: Option<&str>,
    files: &[String],
    rationale: &str,
    intent: Option<&str>,
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
            language,
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
            format_full_message(config, subject, files, rationale, language)
        }
        _ => {
            let subject = if let Some(scope) = scope.filter(|scope| !scope.is_empty()) {
                format!("{commit_type}({scope}): {summary}")
            } else {
                format!("{commit_type}: {summary}")
            };
            format_full_message(config, subject, files, rationale, language)
        }
    }
}

fn format_full_message(
    config: &ResolvedConfig,
    subject: String,
    files: &[String],
    rationale: &str,
    language: &str,
) -> String {
    let mut message = subject;
    if config.commit.include_body && !files.is_empty() && config.commit.format != "simple" {
        message.push_str("\n\n");
        message.push_str(&build_body(files, rationale, language));
    }
    if config.commit.include_footer && !files.is_empty() && config.commit.format != "simple" {
        message.push_str("\n\n");
        message.push_str(&build_footer(files));
    }
    message
}

fn build_body(files: &[String], rationale: &str, language: &str) -> String {
    let mut body = String::new();
    match language {
        "zh" => {
            body.push_str("涉及文件:\n");
            for file in files {
                body.push_str("- ");
                body.push_str(file);
                body.push('\n');
            }
            body.push_str("\n分组依据:\n- ");
            body.push_str(rationale);
        }
        _ => {
            body.push_str("Affected files:\n");
            for file in files {
                body.push_str("- ");
                body.push_str(file);
                body.push('\n');
            }
            body.push_str("\nPlanner rationale:\n- ");
            body.push_str(rationale);
        }
    }
    body
}

fn build_footer(files: &[String]) -> String {
    format!("Files: {}", files.join(", "))
}

fn default_summary(scope: Option<&str>, files: &[String], language: &str) -> String {
    match language {
        "zh" => match scope {
            Some(scope) => format!("更新 {scope} 相关改动"),
            None if !files.is_empty() => format!("更新 {}", files[0]),
            None => "更新改动".to_string(),
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
        "zh" | "zh-cn" | "zh-hans" | "chinese" | "中文" => "zh",
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
