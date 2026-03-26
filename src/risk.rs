use crate::cli::CommandKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    High,
}

impl RiskLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RiskDecision {
    pub level: RiskLevel,
    pub reason: &'static str,
}

pub fn classify(command: &CommandKind) -> RiskDecision {
    match command {
        CommandKind::Merge { .. } => high("merge changes branch history and may apply AI edits"),
        CommandKind::Rebase { .. } => high("rebase rewrites history and may apply AI edits"),
        CommandKind::Sync { .. } => high("sync updates local history from remote"),
        CommandKind::Rollback { .. } => high("rollback resets the working tree to a saved ref"),
        CommandKind::External(args) if external_high_risk(args) => {
            high("external git command matches a dangerous operation")
        }
        _ => low("command is safe enough to run without extra confirmation"),
    }
}

fn external_high_risk(args: &[String]) -> bool {
    let first = args.first().map(String::as_str).unwrap_or_default();
    matches!(
        first,
        "merge" | "rebase" | "reset" | "clean" | "pull" | "push"
    ) || args.iter().any(|arg| arg == "--force" || arg == "-f")
}

fn high(reason: &'static str) -> RiskDecision {
    RiskDecision {
        level: RiskLevel::High,
        reason,
    }
}

fn low(reason: &'static str) -> RiskDecision {
    RiskDecision {
        level: RiskLevel::Low,
        reason,
    }
}
