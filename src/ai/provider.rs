use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

use crate::events::Emitter;

use super::AiConfig;
use super::commit_plan_tool::{extract_commit_plan_tool_args, extract_resolve_conflicts_tool_args};
use super::config::resolve_api_key;
use super::exchange::{AgentResponse, ProviderExchange};
use super::request::{AgentRequest, AgentTask};

pub(super) trait AiProvider {
    fn execute<'a>(
        &'a self,
        config: &'a AiConfig,
        request: &'a AgentRequest,
        provider_request: Value,
        emitter: &'a mut Emitter,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderExchange>> + Send + 'a>>;
}

#[derive(Debug, Clone)]
pub(super) struct OpenAiCompatProvider {
    pub(super) http: Client,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChatCompletionResponse {
    pub(super) choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Choice {
    pub(super) message: Message,
}

#[derive(Debug, Deserialize)]
pub(super) struct Message {
    #[serde(default)]
    pub(super) tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ToolCall {
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) function: ToolFunction,
}

#[derive(Debug, Deserialize)]
pub(super) struct ToolFunction {
    pub(super) name: String,
    pub(super) arguments: String,
}

impl AiProvider for OpenAiCompatProvider {
    fn execute<'a>(
        &'a self,
        config: &'a AiConfig,
        request: &'a AgentRequest,
        provider_request: Value,
        _emitter: &'a mut Emitter,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderExchange>> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
            let response = self
                .http
                .post(url)
                .bearer_auth(resolve_api_key(config)?)
                .json(&provider_request)
                .send()
                .await?;
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            if matches!(
                request.task,
                AgentTask::PlanCommit | AgentTask::ResolveConflicts
            ) && content_type.contains("text/event-stream")
            {
                return Err(anyhow!(
                    "provider does not support tool calls for this task"
                ));
            }
            let provider_response = response
                .json::<Value>()
                .await
                .context("failed to decode AI response JSON")?;
            let response: ChatCompletionResponse =
                serde_json::from_value(provider_response.clone())
                    .context("failed to decode chat completion response")?;
            let agent_response = match request.task {
                AgentTask::ResolveConflicts => {
                    AgentResponse::Patch(extract_resolve_conflicts_tool_args(&response)?)
                }
                AgentTask::PlanCommit => {
                    AgentResponse::CommitPlan(extract_commit_plan_tool_args(&response)?)
                }
            };
            Ok(ProviderExchange {
                provider_response,
                agent_response,
            })
        })
    }
}
