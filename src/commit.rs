use crate::config::{CommitScope, ResolvedConfig};
use crate::git::GitSnapshot;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const AUTO_SPLIT_MIN_CONFIDENCE: f32 = 0.85;
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
    pub grouping_decision: GroupingDecision,
    pub grouping_confidence: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub single_group: Option<CommitGroup>,
    pub groups: Vec<CommitGroup>,
    pub confidence: f32,
    pub warnings: Vec<String>,
    pub auto_executable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GroupingDecision {
    Single,
    Split,
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

impl CommitPlan {
    pub fn normalize_for_execution(mut self, config: &ResolvedConfig) -> Self {
        self.confidence = self.confidence.clamp(0.0, 1.0);
        self.grouping_confidence = self.grouping_confidence.clamp(0.0, 1.0);
        self.groups = self
            .groups
            .into_iter()
            .map(|group| normalize_group_message(group, config))
            .collect();
        self.single_group = self
            .single_group
            .take()
            .map(|group| normalize_group_message(group, config));
        if self.single_group.is_none() && !self.groups.is_empty() {
            self.single_group = Some(normalize_group_message(
                self.merge_into_single_group(),
                config,
            ));
        }

        if !self.should_use_split_plan() {
            if self.groups.len() > 1 {
                self.warnings.push(
                    "grouping confidence below threshold; collapsed to single commit".to_string(),
                );
                if let Some(single_group) = self.single_group.clone() {
                    self.groups = vec![single_group];
                }
            } else if self.groups.is_empty() {
                if let Some(single_group) = self.single_group.clone() {
                    self.groups = vec![single_group];
                }
            }
        }

        self.auto_executable = !self.groups.is_empty();
        self
    }

    fn should_use_split_plan(&self) -> bool {
        self.grouping_decision == GroupingDecision::Split
            && self.grouping_confidence >= AUTO_SPLIT_MIN_CONFIDENCE
            && self.groups.len() > 1
    }

    fn merge_into_single_group(&self) -> CommitGroup {
        let mut files = self
            .groups
            .iter()
            .flat_map(|group| group.files.iter().cloned())
            .collect::<Vec<_>>();
        files.sort();
        files.dedup();

        let commit_message = self
            .groups
            .first()
            .map(|group| group.commit_message.clone())
            .unwrap_or_else(|| "feat: update changes".to_string());
        CommitGroup {
            scope: None,
            files,
            commit_message,
            rationale: "collapsed split plan into a single commit".to_string(),
        }
    }
}

fn normalize_group_message(group: CommitGroup, config: &ResolvedConfig) -> CommitGroup {
    let summary_hint = extract_summary_hint(&group.commit_message);
    let commit_message = format_message(
        config,
        group.scope.as_deref(),
        &group.files,
        &group.rationale,
        summary_hint.as_deref(),
    );
    CommitGroup {
        commit_message,
        ..group
    }
}

fn extract_summary_hint(message: &str) -> Option<String> {
    let subject = message.lines().next()?.trim();
    if subject.is_empty() {
        return None;
    }
    if let Some((_, summary)) = subject.split_once(": ") {
        return Some(summary.trim().to_string());
    }
    if let Some((_, summary)) = subject.split_once(' ') {
        return Some(summary.trim().to_string());
    }
    Some(subject.to_string())
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
    _rationale: &str,
    _language: &str,
) -> String {
    let mut message = subject;
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
