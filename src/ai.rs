mod client;
mod commit_plan_tool;
mod config;
mod context;
mod diff_summary;
mod exchange;
mod provider;
mod request;

pub use config::AiConfig;
pub(crate) use context::collect_repo_context;
pub use exchange::AiPatch;
use provider::OpenAiCompatProvider;

pub struct AiClient {
    config: AiConfig,
    provider: OpenAiCompatProvider,
}
