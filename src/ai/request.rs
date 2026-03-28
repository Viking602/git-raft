use super::commit_plan_tool::{commit_plan_tool_definition, resolve_conflicts_tool_definition};
use anyhow::Result;
use serde::Serialize;
use serde_json::{Value, json};
use std::hash::{Hash, Hasher};

use crate::git::{DiffStat, GitSnapshot};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentTask {
    ResolveConflicts,
    PlanCommit,
}

impl AgentTask {
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::ResolveConflicts => "resolve_conflicts",
            Self::PlanCommit => "plan_commit",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RepoContextPayload {
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub git_snapshot: GitSnapshot,
    pub diff_stats: Vec<DiffStat>,
    pub recent_subjects: Vec<String>,
    pub commit_format: String,
    pub commit_language: String,
    pub commit_use_gitmoji: bool,
    pub commit_include_body: bool,
    pub commit_include_footer: bool,
    pub commit_ignore_paths: Vec<String>,
    pub commit_examples_file: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub commit_examples: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AgentRequest {
    pub task: AgentTask,
    pub system_prompt: String,
    pub user_payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_context: Option<RepoContextPayload>,
}

impl AgentRequest {
    pub(crate) fn task_name(&self) -> &'static str {
        self.task.as_str()
    }

    pub(crate) fn summary(&self) -> Value {
        let conflict_files = self
            .user_payload
            .get("conflicts")
            .and_then(Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(|file| file.get("path").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let attempt = self
            .user_payload
            .get("attempt")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        json!({
            "task": self.task_name(),
            "attempt": attempt,
            "hasRepoContext": self.repo_context.is_some(),
            "branch": self.repo_context.as_ref().and_then(|ctx| ctx.branch.clone()),
            "conflictFiles": conflict_files,
            "recentSubjectCount": self.repo_context.as_ref().map_or(0, |ctx| ctx.recent_subjects.len()),
            "diffStatCount": self.repo_context.as_ref().map_or(0, |ctx| ctx.diff_stats.len()),
        })
    }

    pub(crate) fn provider_request(&self, model: &str) -> Result<Value> {
        let mut request = json!({
            "model": model,
            "messages": [
                {
                    "role": "system",
                    "content": self.system_prompt,
                },
                {
                    "role": "user",
                    "content": serde_json::to_string_pretty(&json!({
                        "task": self.task_name(),
                        "user_payload": self.user_payload,
                        "repo_context": self.repo_context,
                    }))?,
                }
            ]
        });
        if let Some(object) = request.as_object_mut() {
            match self.task {
                AgentTask::ResolveConflicts => {
                    object.insert("temperature".to_string(), json!(0.0));
                    object.insert(
                        "tools".to_string(),
                        Value::Array(vec![resolve_conflicts_tool_definition()]),
                    );
                    object.insert(
                        "tool_choice".to_string(),
                        json!({
                            "type": "function",
                            "function": {
                                "name": "resolve_conflicts"
                            }
                        }),
                    );
                }
                AgentTask::PlanCommit => {
                    object.insert("temperature".to_string(), json!(0.0));
                    object.insert(
                        "tools".to_string(),
                        Value::Array(vec![commit_plan_tool_definition()]),
                    );
                    object.insert(
                        "tool_choice".to_string(),
                        json!({
                            "type": "function",
                            "function": {
                                "name": "plan_commit"
                            }
                        }),
                    );
                }
            }
        }
        Ok(request)
    }

    pub(crate) fn cache_fingerprint(
        &self,
        base_url: &str,
        model: &str,
        change_set_fingerprint: &str,
    ) -> Result<String> {
        let material = serde_json::to_vec(&json!({
            "base_url": base_url,
            "model": model,
            "task": self.task_name(),
            "user_payload": self.user_payload,
            "change_set_fingerprint": change_set_fingerprint,
        }))?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        material.hash(&mut hasher);
        Ok(format!("{:016x}", hasher.finish()))
    }
}
