mod defaults;
mod files;
mod merge;
mod types;

pub(crate) use files::resolve_config;
pub(crate) use types::{
    CommitConfig, ExternalHookConfig, ResolvedConfig, VerificationCommandConfig,
};
