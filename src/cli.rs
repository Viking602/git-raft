use clap::{Parser, Subcommand, ValueEnum};

/// Run git-raft commands with tracing, hooks, and risk checks.
#[derive(Parser, Debug)]
#[command(
    name = "git-raft",
    version,
    about = "Git-oriented CLI agent with traces and risk gates"
)]
pub struct Cli {
    /// Emit newline-delimited JSON events instead of human-readable output.
    #[arg(long, global = true)]
    pub json: bool,

    /// Skip confirmation prompts for high-risk operations.
    #[arg(long, global = true)]
    pub yes: bool,

    /// Command to execute.
    #[command(subcommand)]
    pub command: CommandKind,
}

#[derive(Subcommand, Debug, Clone)]
pub enum CommandKind {
    /// Ask the AI planner to group changes and create commits.
    Commit {
        /// Print the planned commit groups without creating commits.
        #[arg(long)]
        plan: bool,
        /// Preview the planned commit execution without creating commits.
        #[arg(long)]
        dry_run: bool,
        /// Extra guidance passed to the AI commit planner.
        #[arg(long)]
        intent: Option<String>,
        /// Override the configured commit subject language for this run.
        #[arg(long = "lang", value_enum, value_name = "LANGUAGE")]
        lang: Option<CommitLanguageArg>,
        /// Accepted after -- for compatibility; currently ignored by the commit planner.
        #[arg(
            allow_hyphen_values = true,
            trailing_var_arg = true,
            value_name = "ARGS"
        )]
        args: Vec<String>,
    },
    /// Create and switch to a new branch from a commit.
    Branch {
        /// New branch name.
        #[arg(value_name = "NAME")]
        name: String,
        /// Commit, short SHA, or ref to branch from.
        #[arg(value_name = "COMMIT")]
        target: String,
    },
    /// Run git merge and optionally ask AI to resolve conflicts.
    Merge {
        /// Branch, commit, or ref to merge into the current branch.
        #[arg(value_name = "TARGET")]
        target: String,
        /// Try AI conflict resolution when merge stops on conflicts.
        #[arg(long, default_value_t = true)]
        apply_ai: bool,
        /// Extra arguments passed to git merge.
        #[arg(
            allow_hyphen_values = true,
            trailing_var_arg = true,
            value_name = "GIT_ARGS"
        )]
        args: Vec<String>,
    },
    /// Run git rebase and optionally ask AI to resolve conflicts.
    Rebase {
        /// Branch, commit, or ref to rebase onto.
        #[arg(value_name = "TARGET")]
        target: String,
        /// Try AI conflict resolution when rebase stops on conflicts.
        #[arg(long, default_value_t = true)]
        apply_ai: bool,
        /// Extra arguments passed to git rebase.
        #[arg(
            allow_hyphen_values = true,
            trailing_var_arg = true,
            value_name = "GIT_ARGS"
        )]
        args: Vec<String>,
    },
    /// Remove files from the branch and rewrite history to erase them completely.
    Purge {
        /// File or directory paths to remove from the branch and its entire history.
        #[arg(required = true, num_args = 1.., value_name = "PATHS")]
        paths: Vec<String>,
        /// Allow rewriting commits that have already been pushed to the remote.
        #[arg(long)]
        force: bool,
        /// Force push to remote after rewriting (requires --force).
        #[arg(long)]
        push: bool,
    },
    /// Set project-level commit author and rewrite recent commits with wrong author.
    Author {
        /// Author name for this project.
        #[arg(long)]
        name: String,
        /// Author email for this project.
        #[arg(long)]
        email: String,
        /// Allow rewriting commits that have already been pushed to the remote.
        #[arg(long)]
        force: bool,
        /// Force push to remote after rewriting (requires --force).
        #[arg(long)]
        push: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CommitLanguageArg {
    #[value(help = "Generate commit subjects in English.")]
    En,
    #[value(help = "Generate commit subjects in Chinese.")]
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
            Self::Commit { .. } => "commit",
            Self::Branch { .. } => "branch",
            Self::Merge { .. } => "merge",
            Self::Rebase { .. } => "rebase",
            Self::Purge { .. } => "purge",
            Self::Author { .. } => "author",
        }
    }
}
