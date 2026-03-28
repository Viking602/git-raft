use super::{
    ExternalHookConfig, HookConfigSnapshot, HookContext, HookDecision, HookPayload, HookResult,
};
use anyhow::{Context, Result, anyhow};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) async fn run_external_hook(
    hook: &ExternalHookConfig,
    context: &HookContext<'_>,
) -> Result<HookDecision> {
    if hook.program.trim().is_empty() {
        return Err(anyhow!("external hook program is empty"));
    }

    let payload = HookPayload {
        event: context.event.to_string(),
        command: context.command.to_string(),
        repo_root: context.repo.root_dir.display().to_string(),
        cwd: context.cwd.display().to_string(),
        timestamp_ms: now_ms(),
        config: HookConfigSnapshot {
            provider_model: context.config.provider.model.clone(),
            provider_base_url: context.config.provider.base_url.clone(),
            provider_api_key_env: context.config.provider.api_key_env.clone(),
            commit_format: context.config.commit.format.clone(),
            commit_examples_file: context.config.commit.examples_file.clone(),
            runs_dir: context.config.runs.dir.clone(),
        },
        git_snapshot: context.git_snapshot.clone(),
        intent: context.intent.map(str::to_string),
        commit_plan: context.commit_plan.cloned(),
        commit_group: context.commit_group.cloned(),
        commit_message: context.commit_message.map(str::to_string),
        agent_task: context.agent_task.map(str::to_string),
        agent_request_summary: context.agent_request_summary.cloned(),
        agent_response_summary: context.agent_response_summary.cloned(),
        patch_confidence: context.patch_confidence,
    };

    let mut command = Command::new(&hook.program);
    command
        .args(&hook.args)
        .current_dir(&context.repo.root_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn hook {}", hook.program))?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(serde_json::to_string(&payload)?.as_bytes())
            .await?;
    }
    let output = child.wait_with_output().await?;
    let stdout = String::from_utf8(output.stdout).unwrap_or_default();
    let parsed = if stdout.trim().is_empty() {
        HookResult::default()
    } else {
        serde_json::from_str::<HookResult>(&stdout).unwrap_or_default()
    };

    let mut decision = HookDecision::allow();
    if !output.status.success() || parsed.blocked.unwrap_or(false) {
        block(
            &mut decision,
            parsed
                .reason
                .unwrap_or_else(|| format!("hook `{}` blocked the command", hook.program)),
        );
    }
    if let Some(warnings) = parsed.warnings {
        decision.warnings = warnings;
    }
    decision.commit_message = parsed.commit_message;
    Ok(decision)
}

pub(super) fn merge_decisions(target: &mut HookDecision, incoming: HookDecision) {
    if incoming.blocked {
        target.allowed = false;
        target.blocked = true;
        target.reason = incoming.reason;
    }
    if !incoming.warnings.is_empty() {
        target.warnings.extend(incoming.warnings);
    }
    if incoming.commit_message.is_some() {
        target.commit_message = incoming.commit_message;
    }
}

fn block(decision: &mut HookDecision, reason: impl Into<String>) {
    decision.allowed = false;
    decision.blocked = true;
    decision.reason = Some(reason.into());
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_millis()
}
