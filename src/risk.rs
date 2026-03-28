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
        _ => low("command is safe enough to run without extra confirmation"),
    }
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
