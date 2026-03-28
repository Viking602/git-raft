use crate::commit::CommitPlanningInputs;
use crate::config::ResolvedConfig;
use crate::git::GitSnapshot;

pub(super) fn collect_planning_inputs(
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

fn should_ignore_file(file: &str, custom_rules: &[String]) -> bool {
    let normalized = normalize_rule(file);
    if normalized.starts_with(".config/git-raft/") {
        return true;
    }
    if super::DEFAULT_IGNORED_TOOL_DIRS
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
