mod inputs;
mod message;

use crate::config::ResolvedConfig;
use crate::git::GitSnapshot;
use message::normalize_group_message;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

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
    pub fn retain_changed_files(mut self, changed_files: &[String]) -> Self {
        let allowed = changed_files.iter().cloned().collect::<BTreeSet<_>>();
        let mut removed = BTreeSet::new();

        self.groups = self
            .groups
            .into_iter()
            .filter_map(|mut group| {
                group.files = retain_group_files(group.files, &allowed, &mut removed);
                (!group.files.is_empty()).then_some(group)
            })
            .collect();

        self.single_group = self.single_group.take().and_then(|mut group| {
            group.files = retain_group_files(group.files, &allowed, &mut removed);
            (!group.files.is_empty()).then_some(group)
        });

        if !removed.is_empty() {
            self.warnings.push(format!(
                "removed files not present in current change set: {}",
                removed.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }

        self
    }

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

fn retain_group_files(
    files: Vec<String>,
    allowed: &BTreeSet<String>,
    removed: &mut BTreeSet<String>,
) -> Vec<String> {
    let mut kept = Vec::new();
    for file in files {
        if allowed.contains(&file) {
            kept.push(file);
        } else {
            removed.insert(file);
        }
    }
    kept.sort();
    kept.dedup();
    kept
}

pub fn collect_planning_inputs(
    snapshot: &GitSnapshot,
    config: &ResolvedConfig,
) -> CommitPlanningInputs {
    inputs::collect_planning_inputs(snapshot, config)
}
