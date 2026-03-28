use crate::ai::AiPatch;
use crate::config::VerificationCommandConfig;
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use uuid::Uuid;

use super::retention::{MissingUniqueBlock, MissingUniqueLine, RetentionCheck};

pub(crate) const NO_VERIFICATION_COMMANDS: &str = "merge verification commands are not configured";
pub(crate) const NON_TEXT_CONFLICT: &str = "conflict files must be decodable text";
pub(crate) const VALIDATION_COMMANDS_FAILED: &str = "merge verification commands failed";

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValidationTrace {
    pub(crate) attempts: Vec<ValidationAttemptRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValidationAttemptRecord {
    pub(crate) attempt: usize,
    pub(crate) validation_passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rejection_reason: Option<String>,
    pub(crate) missing_unique_lines: Vec<MissingUniqueLine>,
    pub(crate) missing_unique_blocks: Vec<MissingUniqueBlock>,
    pub(crate) commands: Vec<ValidationCommandRecord>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ValidationCommandRecord {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) passed: bool,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

impl ValidationAttemptRecord {
    pub(crate) fn non_text_failure(reason: impl Into<String>) -> Self {
        Self {
            attempt: 0,
            validation_passed: false,
            rejection_reason: Some(reason.into()),
            missing_unique_lines: Vec::new(),
            missing_unique_blocks: Vec::new(),
            commands: Vec::new(),
        }
    }

    pub(crate) fn from_retention(attempt: usize, retention: &RetentionCheck) -> Self {
        Self {
            attempt,
            validation_passed: retention.passed,
            rejection_reason: retention.rejection_reason.clone(),
            missing_unique_lines: retention.missing_unique_lines.clone(),
            missing_unique_blocks: retention.missing_unique_blocks.clone(),
            commands: Vec::new(),
        }
    }

    pub(crate) fn config_missing(attempt: usize) -> Self {
        Self {
            attempt,
            validation_passed: false,
            rejection_reason: Some(NO_VERIFICATION_COMMANDS.to_string()),
            missing_unique_lines: Vec::new(),
            missing_unique_blocks: Vec::new(),
            commands: Vec::new(),
        }
    }

    pub(crate) fn is_repairable(&self) -> bool {
        matches!(
            self.rejection_reason.as_deref(),
            Some("AI candidate file set does not match the conflict files")
                | Some("AI candidate still contains conflict markers")
                | Some("AI candidate removed unique conflict content")
                | Some("failed to parse conflict markers for conflict.txt")
                | Some(VALIDATION_COMMANDS_FAILED)
        ) || self
            .rejection_reason
            .as_deref()
            .unwrap_or_default()
            .starts_with("failed to parse conflict markers for ")
    }

    pub(crate) fn repair_context(&self, patch: &AiPatch) -> Value {
        json!({
            "attempt": self.attempt,
            "rejection_reason": self.rejection_reason,
            "missing_unique_lines": self.missing_unique_lines,
            "missing_unique_blocks": self.missing_unique_blocks,
            "commands": self.commands,
            "previous_candidate": patch,
        })
    }
}

pub(crate) async fn run_validation_commands(
    repo_root: &Path,
    patch: &AiPatch,
    commands: &[VerificationCommandConfig],
    attempt: usize,
) -> Result<ValidationAttemptRecord> {
    if commands.is_empty() {
        return Ok(ValidationAttemptRecord::config_missing(attempt));
    }

    let scratch = ScratchDir::new()?;
    copy_tree(repo_root, scratch.path())?;
    write_patch_files(scratch.path(), patch)?;

    let mut records = Vec::new();
    for command in commands {
        let output = Command::new(&command.program)
            .args(&command.args)
            .current_dir(scratch.path())
            .output()
            .await
            .with_context(|| format!("failed to run validation command {}", command.program))?;
        let record = ValidationCommandRecord {
            program: command.program.clone(),
            args: command.args.clone(),
            exit_code: output.status.code(),
            passed: output.status.success(),
            stdout: truncate_output(&String::from_utf8_lossy(&output.stdout)),
            stderr: truncate_output(&String::from_utf8_lossy(&output.stderr)),
        };
        let passed = record.passed;
        records.push(record);
        if !passed {
            return Ok(ValidationAttemptRecord {
                attempt,
                validation_passed: false,
                rejection_reason: Some(VALIDATION_COMMANDS_FAILED.to_string()),
                missing_unique_lines: Vec::new(),
                missing_unique_blocks: Vec::new(),
                commands: records,
            });
        }
    }

    Ok(ValidationAttemptRecord {
        attempt,
        validation_passed: true,
        rejection_reason: None,
        missing_unique_lines: Vec::new(),
        missing_unique_blocks: Vec::new(),
        commands: records,
    })
}

struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("git-raft-validate-{}", Uuid::new_v4()));
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_name = entry.file_name();
        if file_name == ".git" {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(&file_name);
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            fs::create_dir_all(&destination_path)?;
            copy_tree(&source_path, &destination_path)?;
            continue;
        }
        if file_type.is_file() {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source_path, &destination_path)?;
            continue;
        }
        if file_type.is_symlink() {
            let target = fs::read_link(&source_path)?;
            let resolved = if target.is_absolute() {
                target
            } else {
                source_path
                    .parent()
                    .unwrap_or(source)
                    .join(target)
                    .canonicalize()?
            };
            if resolved.is_dir() {
                fs::create_dir_all(&destination_path)?;
                copy_tree(&resolved, &destination_path)?;
            } else {
                if let Some(parent) = destination_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&resolved, &destination_path)?;
            }
        }
    }
    Ok(())
}

fn write_patch_files(root: &Path, patch: &AiPatch) -> Result<()> {
    for file in &patch.files {
        let path = root.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, &file.resolved_content)?;
    }
    Ok(())
}

fn truncate_output(output: &str) -> String {
    const LIMIT: usize = 4_000;
    let trimmed = output.trim();
    let mut chars = trimmed.chars();
    let preview = chars.by_ref().take(LIMIT).collect::<String>();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}
