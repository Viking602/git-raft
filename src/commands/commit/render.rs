use crate::commit;
use crate::hooks;

pub(super) fn render_commit_plan_summary(
    plan: &commit::CommitPlan,
    hook: &hooks::HookDecision,
) -> String {
    let mut out = String::new();
    out.push_str("Commit plan\n");
    out.push_str(&format!("decision: {:?}\n", plan.grouping_decision).to_lowercase());
    out.push_str(&format!("confidence: {:.2}\n", plan.confidence));
    out.push_str(&format!(
        "grouping confidence: {:.2}\n",
        plan.grouping_confidence
    ));
    if !plan.warnings.is_empty() || !hook.warnings.is_empty() {
        out.push_str("warnings:\n");
        for warning in &plan.warnings {
            out.push_str("- ");
            out.push_str(warning);
            out.push('\n');
        }
        for warning in &hook.warnings {
            out.push_str("- ");
            out.push_str(warning);
            out.push('\n');
        }
    }
    for (index, group) in plan.groups.iter().enumerate() {
        out.push_str(&format!("Commit {}\n", index + 1));
        out.push_str("message: ");
        out.push_str(&group.commit_message);
        out.push('\n');
        out.push_str("rationale: ");
        out.push_str(&group.rationale);
        out.push('\n');
        out.push_str("files:\n");
        for file in &group.files {
            out.push_str("- ");
            out.push_str(file);
            out.push('\n');
        }
    }
    if let Some(reason) = &hook.reason {
        out.push_str("blocked reason: ");
        out.push_str(reason);
        out.push('\n');
    }
    out.trim_end().to_string()
}
