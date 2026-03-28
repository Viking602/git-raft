use crate::ai::AiPatch;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub(crate) struct ConflictTextFile {
    pub(crate) path: String,
    pub(crate) current: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MissingUniqueLine {
    pub(crate) path: String,
    pub(crate) side: String,
    pub(crate) line: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MissingUniqueBlock {
    pub(crate) path: String,
    pub(crate) side: String,
    pub(crate) lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequiredUniqueLine {
    pub(crate) path: String,
    pub(crate) side: String,
    pub(crate) line: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequiredUniqueBlock {
    pub(crate) path: String,
    pub(crate) side: String,
    pub(crate) lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreservationRequirements {
    pub(crate) required_unique_lines: Vec<RequiredUniqueLine>,
    pub(crate) required_unique_blocks: Vec<RequiredUniqueBlock>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetentionCheck {
    pub(crate) passed: bool,
    pub(crate) rejection_reason: Option<String>,
    pub(crate) missing_unique_lines: Vec<MissingUniqueLine>,
    pub(crate) missing_unique_blocks: Vec<MissingUniqueBlock>,
}

impl RetentionCheck {
    pub(crate) fn pass() -> Self {
        Self {
            passed: true,
            rejection_reason: None,
            missing_unique_lines: Vec::new(),
            missing_unique_blocks: Vec::new(),
        }
    }

    pub(crate) fn reject(reason: impl Into<String>) -> Self {
        Self {
            passed: false,
            rejection_reason: Some(reason.into()),
            missing_unique_lines: Vec::new(),
            missing_unique_blocks: Vec::new(),
        }
    }
}

pub(crate) fn validate_patch(conflicts: &[ConflictTextFile], patch: &AiPatch) -> RetentionCheck {
    let expected_paths = conflicts
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    let actual_paths = patch
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<Vec<_>>();
    let actual_set = actual_paths.iter().copied().collect::<BTreeSet<_>>();
    if actual_paths.len() != actual_set.len() || actual_set != expected_paths {
        return RetentionCheck::reject("AI candidate file set does not match the conflict files");
    }

    let mut missing_unique_lines = Vec::new();
    let mut missing_unique_blocks = Vec::new();
    for conflict in conflicts {
        let Some(candidate) = patch.files.iter().find(|file| file.path == conflict.path) else {
            return RetentionCheck::reject("AI candidate omitted a conflicted file");
        };
        if contains_conflict_markers(&candidate.resolved_content) {
            return RetentionCheck::reject("AI candidate still contains conflict markers");
        }

        let Ok(blocks) = parse_conflict_blocks(&conflict.current) else {
            return RetentionCheck::reject(format!(
                "failed to parse conflict markers for {}",
                conflict.path
            ));
        };
        let candidate_lines = candidate
            .resolved_content
            .lines()
            .filter_map(normalize_line)
            .collect::<Vec<_>>();
        let candidate_set = candidate_lines.iter().cloned().collect::<BTreeSet<_>>();

        for block in &blocks {
            let ours_unique = unique_lines(&block.ours, &block.theirs);
            let theirs_unique = unique_lines(&block.theirs, &block.ours);

            for line in &ours_unique {
                if !candidate_set.contains(line) {
                    missing_unique_lines.push(MissingUniqueLine {
                        path: conflict.path.clone(),
                        side: "ours".to_string(),
                        line: line.clone(),
                    });
                }
            }
            for line in &theirs_unique {
                if !candidate_set.contains(line) {
                    missing_unique_lines.push(MissingUniqueLine {
                        path: conflict.path.clone(),
                        side: "theirs".to_string(),
                        line: line.clone(),
                    });
                }
            }

            for block_lines in unique_blocks(&block.ours, &ours_unique) {
                if !contains_block(&candidate_lines, &block_lines) {
                    missing_unique_blocks.push(MissingUniqueBlock {
                        path: conflict.path.clone(),
                        side: "ours".to_string(),
                        lines: block_lines,
                    });
                }
            }
            for block_lines in unique_blocks(&block.theirs, &theirs_unique) {
                if !contains_block(&candidate_lines, &block_lines) {
                    missing_unique_blocks.push(MissingUniqueBlock {
                        path: conflict.path.clone(),
                        side: "theirs".to_string(),
                        lines: block_lines,
                    });
                }
            }
        }
    }

    if missing_unique_lines.is_empty() && missing_unique_blocks.is_empty() {
        RetentionCheck::pass()
    } else {
        RetentionCheck {
            passed: false,
            rejection_reason: Some("AI candidate removed unique conflict content".to_string()),
            missing_unique_lines,
            missing_unique_blocks,
        }
    }
}

pub(crate) fn preservation_requirements(
    conflicts: &[ConflictTextFile],
) -> Result<PreservationRequirements, String> {
    let mut required_unique_lines = Vec::new();
    let mut required_unique_blocks = Vec::new();

    for conflict in conflicts {
        let blocks = parse_conflict_blocks(&conflict.current)
            .map_err(|_| format!("failed to parse conflict markers for {}", conflict.path))?;

        for block in &blocks {
            let ours_unique = unique_lines(&block.ours, &block.theirs);
            let theirs_unique = unique_lines(&block.theirs, &block.ours);

            for line in &ours_unique {
                required_unique_lines.push(RequiredUniqueLine {
                    path: conflict.path.clone(),
                    side: "ours".to_string(),
                    line: line.clone(),
                });
            }
            for line in &theirs_unique {
                required_unique_lines.push(RequiredUniqueLine {
                    path: conflict.path.clone(),
                    side: "theirs".to_string(),
                    line: line.clone(),
                });
            }

            for lines in unique_blocks(&block.ours, &ours_unique) {
                required_unique_blocks.push(RequiredUniqueBlock {
                    path: conflict.path.clone(),
                    side: "ours".to_string(),
                    lines,
                });
            }
            for lines in unique_blocks(&block.theirs, &theirs_unique) {
                required_unique_blocks.push(RequiredUniqueBlock {
                    path: conflict.path.clone(),
                    side: "theirs".to_string(),
                    lines,
                });
            }
        }
    }

    Ok(PreservationRequirements {
        required_unique_lines,
        required_unique_blocks,
    })
}

#[derive(Debug, Clone)]
struct ConflictBlock {
    ours: Vec<String>,
    theirs: Vec<String>,
}

fn parse_conflict_blocks(content: &str) -> Result<Vec<ConflictBlock>, ()> {
    #[derive(Clone, Copy)]
    enum State {
        Outside,
        Ours,
        Base,
        Theirs,
    }

    let mut state = State::Outside;
    let mut blocks = Vec::new();
    let mut ours = Vec::new();
    let mut theirs = Vec::new();

    for line in content.lines() {
        if line.starts_with("<<<<<<<") {
            if !matches!(state, State::Outside) {
                return Err(());
            }
            state = State::Ours;
            ours.clear();
            theirs.clear();
            continue;
        }
        if line.starts_with("|||||||") {
            if !matches!(state, State::Ours) {
                return Err(());
            }
            state = State::Base;
            continue;
        }
        if line.starts_with("=======") {
            if !matches!(state, State::Ours | State::Base) {
                return Err(());
            }
            state = State::Theirs;
            continue;
        }
        if line.starts_with(">>>>>>>") {
            if !matches!(state, State::Theirs) {
                return Err(());
            }
            blocks.push(ConflictBlock {
                ours: ours.clone(),
                theirs: theirs.clone(),
            });
            state = State::Outside;
            continue;
        }

        match state {
            State::Outside => {}
            State::Ours => ours.push(line.to_string()),
            State::Base => {}
            State::Theirs => theirs.push(line.to_string()),
        }
    }

    if matches!(state, State::Outside) {
        Ok(blocks)
    } else {
        Err(())
    }
}

fn contains_conflict_markers(content: &str) -> bool {
    content.contains("<<<<<<<") || content.contains("=======") || content.contains(">>>>>>>")
}

fn normalize_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("<<<<<<<")
        || trimmed.starts_with("=======")
        || trimmed.starts_with(">>>>>>>")
        || trimmed.starts_with("|||||||")
    {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn unique_lines(side: &[String], other: &[String]) -> BTreeSet<String> {
    let side_set = side
        .iter()
        .filter_map(|line| normalize_line(line))
        .collect::<BTreeSet<_>>();
    let other_set = other
        .iter()
        .filter_map(|line| normalize_line(line))
        .collect::<BTreeSet<_>>();
    side_set.difference(&other_set).cloned().collect()
}

fn unique_blocks(side: &[String], unique_lines: &BTreeSet<String>) -> Vec<Vec<String>> {
    let mut blocks = Vec::new();
    let mut current = Vec::new();
    for line in side.iter().filter_map(|line| normalize_line(line)) {
        if unique_lines.contains(&line) {
            current.push(line);
            continue;
        }
        if current.len() >= 3 {
            blocks.push(std::mem::take(&mut current));
        } else {
            current.clear();
        }
    }
    if current.len() >= 3 {
        blocks.push(current);
    }
    blocks
}

fn contains_block(candidate_lines: &[String], block: &[String]) -> bool {
    if block.is_empty() || block.len() > candidate_lines.len() {
        return false;
    }
    candidate_lines
        .windows(block.len())
        .any(|window| window == block)
}

#[cfg(test)]
mod tests {
    use super::{ConflictTextFile, preservation_requirements, validate_patch};
    use crate::ai::AiPatch;
    use serde_json::json;

    fn patch(content: &str) -> AiPatch {
        serde_json::from_value(json!({
            "confidence": 0.95,
            "summary": "resolved conflict",
            "files": [
                {
                    "path": "conflict.txt",
                    "explanation": "resolved",
                    "resolved_content": content,
                }
            ]
        }))
        .expect("patch")
    }

    fn conflict(current: &str) -> ConflictTextFile {
        ConflictTextFile {
            path: "conflict.txt".to_string(),
            current: current.to_string(),
        }
    }

    #[test]
    fn rejects_missing_unique_lines() {
        let conflict =
            conflict("<<<<<<< ours\nalpha\nbeta\n=======\nalpha\ngamma\n>>>>>>> theirs\n");
        let result = validate_patch(&[conflict], &patch("alpha\nbeta\n"));
        assert!(!result.passed);
        assert_eq!(result.missing_unique_lines.len(), 1);
        assert_eq!(result.missing_unique_lines[0].line, "gamma");
    }

    #[test]
    fn rejects_missing_unique_blocks() {
        let conflict = conflict(
            "<<<<<<< ours\nshared\nkeep_a\nkeep_b\nkeep_c\n=======\nshared\nother\n>>>>>>> theirs\n",
        );
        let result = validate_patch(&[conflict], &patch("shared\nother\nkeep_a\nkeep_c\n"));
        assert!(!result.passed);
        assert_eq!(result.missing_unique_blocks.len(), 1);
    }

    #[test]
    fn accepts_candidate_that_keeps_unique_lines_and_blocks() {
        let conflict = conflict(
            "<<<<<<< ours\nshared\nkeep_a\nkeep_b\nkeep_c\n=======\nshared\nother\n>>>>>>> theirs\n",
        );
        let result = validate_patch(
            &[conflict],
            &patch("shared\nkeep_a\nkeep_b\nkeep_c\nother\n"),
        );
        assert!(result.passed);
    }

    #[test]
    fn extracts_preservation_requirements_from_conflict_markers() {
        let conflict = conflict(
            "<<<<<<< ours\nshared\nkeep_a\nkeep_b\nkeep_c\n=======\nshared\nother\n>>>>>>> theirs\n",
        );
        let result = preservation_requirements(&[conflict]).expect("requirements");
        assert!(
            result
                .required_unique_lines
                .iter()
                .any(|line| line.side == "theirs" && line.line == "other")
        );
        assert!(
            result.required_unique_blocks.iter().any(|block| {
                block.side == "ours"
                    && block.lines == vec!["keep_a", "keep_b", "keep_c"]
            })
        );
    }
}
