use crate::config::{CommitScope, ResolvedConfig};
use crate::git::GitSnapshot;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const AUTO_COMMIT_MIN_CONFIDENCE: f32 = 0.8;

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

pub fn build_plan(
    root_dir: &Path,
    snapshot: &GitSnapshot,
    config: &ResolvedConfig,
    intent: Option<&str>,
) -> Result<CommitPlan> {
    let files = snapshot
        .all_changed_files()
        .into_iter()
        .filter(|file| !file.starts_with(".config/git-raft/"))
        .collect::<Vec<_>>();
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
            let message = format_message(config, scope_opt.as_deref(), &files, intent);
            CommitGroup {
                scope: scope_opt,
                files,
                rationale: format!("grouped by inferred scope `{scope}`"),
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

fn format_message(
    config: &ResolvedConfig,
    scope: Option<&str>,
    files: &[String],
    intent: Option<&str>,
) -> String {
    let summary = intent
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .unwrap_or_else(|| default_summary(scope, files));
    let commit_type = infer_commit_type(intent, files);
    match config.commit.format.as_str() {
        "simple" => capitalize_first(&summary),
        "gitmoji" => format!("{} {}", emoji_for_type(&commit_type), summary),
        "angular" | "conventional" => {
            if let Some(scope) = scope.filter(|scope| !scope.is_empty()) {
                format!("{commit_type}({scope}): {summary}")
            } else {
                format!("{commit_type}: {summary}")
            }
        }
        _ => {
            if let Some(scope) = scope.filter(|scope| !scope.is_empty()) {
                format!("{commit_type}({scope}): {summary}")
            } else {
                format!("{commit_type}: {summary}")
            }
        }
    }
}

fn default_summary(scope: Option<&str>, files: &[String]) -> String {
    match scope {
        Some(scope) => format!("update {scope} changes"),
        None if !files.is_empty() => format!("update {}", files[0]),
        None => "update changes".to_string(),
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
