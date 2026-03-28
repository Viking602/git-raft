use crate::config;
use crate::events::Emitter;
use crate::git;
use crate::hooks::{HookContext, run_hooks};
use anyhow::{Result, anyhow};
use serde_json::json;
use std::path::PathBuf;

pub(crate) async fn run_ai_hook(
    event: &str,
    command: &str,
    cwd: &PathBuf,
    repo: &git::RepoContext,
    resolved_config: &config::ResolvedConfig,
    git_snapshot: &git::GitSnapshot,
    agent_task: Option<&str>,
    agent_request_summary: Option<&serde_json::Value>,
    agent_response_summary: Option<&serde_json::Value>,
    patch_confidence: Option<f32>,
    emitter: &mut Emitter,
) -> Result<()> {
    let decision = run_hooks(HookContext {
        event,
        command,
        repo,
        cwd,
        config: resolved_config,
        git_snapshot,
        intent: None,
        commit_plan: None,
        commit_group: None,
        commit_message: None,
        agent_task,
        agent_request_summary,
        agent_response_summary,
        patch_confidence,
    })
    .await?;
    if decision.blocked {
        let reason = decision
            .reason
            .unwrap_or_else(|| format!("hook blocked {event}"));
        emitter
            .emit(
                "commandFailed",
                Some("ai_wait"),
                Some(reason.clone()),
                Some(json!({ "blocked": true, "event": event })),
            )
            .await?;
        return Err(anyhow!(reason));
    }
    Ok(())
}
