use super::request::{AgentRequest, AgentTask};
use crate::commit::CommitPlan;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiPatch {
    pub confidence: f32,
    pub summary: String,
    pub files: Vec<ResolvedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedFile {
    pub path: String,
    pub explanation: String,
    pub resolved_content: String,
}

#[derive(Debug, Clone)]
pub(super) enum AgentResponse {
    Patch(AiPatch),
    CommitPlan(CommitPlan),
}

#[derive(Debug, Clone)]
pub(super) struct ProviderExchange {
    pub(super) provider_response: Value,
    pub(super) agent_response: AgentResponse,
}

pub(crate) struct AiExchange {
    task: AgentTask,
    request: AgentRequest,
    provider_response: Value,
    response: AgentResponse,
}

impl AiExchange {
    pub(super) fn new(
        task: AgentTask,
        request: AgentRequest,
        provider_response: Value,
        response: AgentResponse,
    ) -> Self {
        Self {
            task,
            request,
            provider_response,
            response,
        }
    }

    pub(crate) fn task_name(&self) -> &'static str {
        self.task.as_str()
    }

    pub(crate) fn request_summary(&self) -> Value {
        self.request.summary()
    }

    pub(crate) fn response_summary(&self) -> Value {
        let attempt = self
            .request
            .user_payload
            .get("attempt")
            .and_then(Value::as_u64)
            .unwrap_or(1);
        match &self.response {
            AgentResponse::Patch(patch) => json!({
                "task": self.task_name(),
                "attempt": attempt,
                "kind": "patch",
                "summary": truncate_text(&patch.summary, 160),
                "patchConfidence": patch.confidence,
                "fileCount": patch.files.len(),
                "files": patch.files.iter().map(|file| file.path.clone()).collect::<Vec<_>>(),
            }),
            AgentResponse::CommitPlan(plan) => json!({
                "task": self.task_name(),
                "attempt": attempt,
                "kind": "commit_plan",
                "groupingDecision": plan.grouping_decision,
                "groupingConfidence": plan.grouping_confidence,
                "groupCount": plan.groups.len(),
                "confidence": plan.confidence,
                "autoExecutable": plan.auto_executable,
                "warnings": plan.warnings,
            }),
        }
    }

    pub(crate) fn patch_confidence(&self) -> Option<f32> {
        match &self.response {
            AgentResponse::Patch(patch) => Some(patch.confidence),
            AgentResponse::CommitPlan(plan) => Some(plan.confidence),
        }
    }

    pub(crate) fn response_record(&self) -> Value {
        json!({
            "task": self.task_name(),
            "response": match &self.response {
                AgentResponse::Patch(patch) => json!({
                    "kind": "patch",
                    "patch": patch,
                }),
                AgentResponse::CommitPlan(plan) => json!({
                    "kind": "commit_plan",
                    "commit_plan": plan,
                }),
            },
            "response_summary": self.response_summary(),
            "provider_response": &self.provider_response,
        })
    }

    pub(crate) fn into_patch(self) -> Result<AiPatch> {
        match self.response {
            AgentResponse::Patch(patch) => Ok(patch),
            AgentResponse::CommitPlan(_) => Err(anyhow!("expected patch AI response")),
        }
    }

    pub(crate) fn into_commit_plan(self) -> Result<CommitPlan> {
        match self.response {
            AgentResponse::CommitPlan(plan) => Ok(plan),
            AgentResponse::Patch(_) => Err(anyhow!("expected commit plan AI response")),
        }
    }
}

fn truncate_text(text: &str, limit: usize) -> String {
    let truncated = text.trim();
    let mut chars = truncated.chars();
    let preview = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}
