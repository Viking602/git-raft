mod builtin;
mod external;

use crate::commit::{CommitGroup, CommitPlan};
use crate::config::{ExternalHookConfig, ResolvedConfig};
use crate::git::{GitSnapshot, RepoContext};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookPayload {
    pub event: String,
    pub command: String,
    pub repo_root: String,
    pub cwd: String,
    pub timestamp_ms: u128,
    pub config: HookConfigSnapshot,
    pub git_snapshot: GitSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_plan: Option<CommitPlan>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_group: Option<CommitGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_task: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_request_summary: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_response_summary: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookConfigSnapshot {
    pub provider_model: String,
    pub provider_base_url: String,
    pub provider_api_key_env: String,
    pub commit_format: String,
    pub commit_examples_file: String,
    pub runs_dir: String,
}

#[derive(Debug, Clone, Default)]
pub struct HookDecision {
    pub allowed: bool,
    pub blocked: bool,
    pub reason: Option<String>,
    pub warnings: Vec<String>,
    pub commit_message: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HookResult {
    blocked: Option<bool>,
    reason: Option<String>,
    warnings: Option<Vec<String>>,
    commit_message: Option<String>,
}

pub struct HookContext<'a> {
    pub event: &'a str,
    pub command: &'a str,
    pub repo: &'a RepoContext,
    pub cwd: &'a PathBuf,
    pub config: &'a ResolvedConfig,
    pub git_snapshot: &'a GitSnapshot,
    pub intent: Option<&'a str>,
    pub commit_plan: Option<&'a CommitPlan>,
    pub commit_group: Option<&'a CommitGroup>,
    pub commit_message: Option<&'a str>,
    pub agent_task: Option<&'a str>,
    pub agent_request_summary: Option<&'a Value>,
    pub agent_response_summary: Option<&'a Value>,
    pub patch_confidence: Option<f32>,
}

impl HookDecision {
    pub fn allow() -> Self {
        Self {
            allowed: true,
            blocked: false,
            reason: None,
            warnings: Vec::new(),
            commit_message: None,
        }
    }
}

pub async fn run_hooks(context: HookContext<'_>) -> Result<HookDecision> {
    let mut decision = builtin::evaluate_builtin_rules(&context)?;
    if decision.blocked {
        return Ok(decision);
    }

    for hook in context
        .config
        .hooks
        .external
        .iter()
        .filter(|hook| hook.event == context.event)
    {
        let external = external::run_external_hook(hook, &context).await?;
        external::merge_decisions(&mut decision, external);
        if decision.blocked {
            break;
        }
    }

    if !decision.blocked {
        decision.allowed = true;
    }
    Ok(decision)
}
