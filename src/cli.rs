use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "git-raft",
    version,
    about = "Git-oriented CLI agent with traces and risk gates"
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub json: bool,

    #[arg(long, global = true)]
    pub yes: bool,

    #[command(subcommand)]
    pub command: CommandKind,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CommandKind {
    Status {
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    Diff {
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    Add {
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    Commit {
        #[arg(long)]
        plan: bool,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        intent: Option<String>,
        #[arg(long, value_enum)]
        language: Option<CommitLanguageArg>,
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    Branch {
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    Switch {
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    Sync {
        #[arg(long)]
        merge: bool,
    },
    Merge {
        target: String,
        #[arg(long, default_value_t = true)]
        apply_ai: bool,
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    Rebase {
        target: String,
        #[arg(long, default_value_t = true)]
        apply_ai: bool,
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    Stash {
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    Log {
        #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
        args: Vec<String>,
    },
    Ask {
        #[arg(required = true)]
        prompt: Vec<String>,
    },
    Init {
        #[arg(long, default_value_t = false)]
        project: bool,
    },
    Rollback {
        run_id: String,
    },
    Runs,
    Trace {
        run_id: Option<String>,
    },
    Doctor,
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Scopes {
        #[command(subcommand)]
        command: ScopesCommand,
    },
    #[command(external_subcommand)]
    External(Vec<String>),
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConfigCommand {
    Show {
        #[arg(long, value_enum, default_value_t = ConfigScopeArg::Resolved)]
        scope: ConfigScopeArg,
    },
    Get {
        key: String,
        #[arg(long, value_enum, default_value_t = ConfigScopeArg::Resolved)]
        scope: ConfigScopeArg,
    },
    Set {
        key: String,
        value: String,
        #[arg(long, value_enum)]
        scope: ConfigWritableScopeArg,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ScopesCommand {
    Generate,
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ConfigScopeArg {
    User,
    Repo,
    Resolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ConfigWritableScopeArg {
    User,
    Repo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CommitLanguageArg {
    En,
    Zh,
}

impl CommitLanguageArg {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Zh => "zh",
        }
    }
}

impl CommandKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Status { .. } => "status",
            Self::Diff { .. } => "diff",
            Self::Add { .. } => "add",
            Self::Commit { .. } => "commit",
            Self::Branch { .. } => "branch",
            Self::Switch { .. } => "switch",
            Self::Sync { .. } => "sync",
            Self::Merge { .. } => "merge",
            Self::Rebase { .. } => "rebase",
            Self::Stash { .. } => "stash",
            Self::Log { .. } => "log",
            Self::Ask { .. } => "ask",
            Self::Init { .. } => "init",
            Self::Rollback { .. } => "rollback",
            Self::Runs => "runs",
            Self::Trace { .. } => "trace",
            Self::Doctor => "doctor",
            Self::Config { .. } => "config",
            Self::Scopes { .. } => "scopes",
            Self::External(_) => "external",
        }
    }
}
