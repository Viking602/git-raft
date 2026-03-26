use crate::commit::{CommitGroup, CommitPlan};
use crate::config::{ExternalHookConfig, ResolvedConfig};
use crate::git::{GitSnapshot, RepoContext};
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

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
    let mut decision = evaluate_builtin_rules(&context)?;
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
        let external = run_external_hook(hook, &context).await?;
        merge_decisions(&mut decision, external);
        if decision.blocked {
            break;
        }
    }

    if !decision.blocked {
        decision.allowed = true;
    }
    Ok(decision)
}

fn evaluate_builtin_rules(context: &HookContext<'_>) -> Result<HookDecision> {
    let rules = &context.config.hooks.rules;
    let mut decision = HookDecision::allow();

    match context.event {
        "afterCommitPlan" => {
            if let Some(plan) = context.commit_plan {
                if rules.empty_group && plan.groups.iter().any(|group| group.files.is_empty()) {
                    block(&mut decision, "empty commit group is not allowed");
                }
                if rules.max_group_count > 0 && plan.groups.len() > rules.max_group_count {
                    block(
                        &mut decision,
                        format!(
                            "commit plan produced {} groups, above max_group_count={}",
                            plan.groups.len(),
                            rules.max_group_count
                        ),
                    );
                }
                if rules.scope_required && plan.groups.iter().any(|group| group.scope.is_none()) {
                    block(&mut decision, "scope is required for every commit group");
                }
            }
        }
        "beforeGroupCommit" => {
            if let Some(group) = context.commit_group {
                if rules.empty_group && group.files.is_empty() {
                    block(&mut decision, "cannot commit an empty group");
                }
                if rules.scope_required && group.scope.is_none() {
                    block(&mut decision, "scope is required for this commit group");
                }
                if rules.validate_message_format {
                    let message = context.commit_message.unwrap_or(&group.commit_message);
                    if !valid_commit_message(message, &context.config.commit.format) {
                        block(
                            &mut decision,
                            format!(
                                "commit message does not match format `{}`",
                                context.config.commit.format
                            ),
                        );
                    }
                }
            }
        }
        _ => {}
    }

    Ok(decision)
}

async fn run_external_hook(
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

fn merge_decisions(target: &mut HookDecision, incoming: HookDecision) {
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

fn valid_commit_message(message: &str, format: &str) -> bool {
    let trimmed = message.trim();
    match format {
        "simple" => !trimmed.is_empty() && !trimmed.contains('\n'),
        "gitmoji" => trimmed.starts_with(':') && trimmed.contains(' '),
        "angular" | "conventional" => {
            let Some((head, subject)) = trimmed.split_once(": ") else {
                return false;
            };
            if subject.trim().is_empty() {
                return false;
            }
            if let Some((ty, scope)) = head.split_once('(') {
                ty.chars().all(|ch| ch.is_ascii_lowercase())
                    && scope.ends_with(')')
                    && !scope.trim_end_matches(')').is_empty()
            } else {
                head.chars().all(|ch| ch.is_ascii_lowercase())
            }
        }
        _ => !trimmed.is_empty(),
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_millis()
}
