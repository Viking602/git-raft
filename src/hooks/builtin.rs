use super::{HookContext, HookDecision};
use anyhow::Result;

pub(super) fn evaluate_builtin_rules(context: &HookContext<'_>) -> Result<HookDecision> {
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
