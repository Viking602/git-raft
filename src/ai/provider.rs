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
        emitter: &'a mut Emitter,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderExchange>> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));

            // Enable streaming
            let mut stream_request = provider_request.clone();
            if let Some(obj) = stream_request.as_object_mut() {
                obj.insert("stream".to_string(), Value::Bool(true));
            }

            let response = self
                .http
                .post(&url)
                .bearer_auth(resolve_api_key(config)?)
                .json(&stream_request)
                .send()
                .await?;

            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();

            // If server responds with SSE stream, parse it
            if content_type.contains("text/event-stream") {
                let assembled = parse_sse_tool_call(response, emitter).await?;
                let provider_response = assembled_to_response(&assembled);
                let chat_response: ChatCompletionResponse =
                    serde_json::from_value(provider_response.clone())
                        .context("failed to decode assembled SSE response")?;
                let agent_response = match request.task {
                    AgentTask::ResolveConflicts => {
                        AgentResponse::Patch(extract_resolve_conflicts_tool_args(&chat_response)?)
                    }
                    AgentTask::PlanCommit => {
                        AgentResponse::CommitPlan(extract_commit_plan_tool_args(&chat_response)?)
                    }
                };
                return Ok(ProviderExchange {
                    provider_response,
                    agent_response,
                });
            }

            // Fallback: non-streaming JSON response
            let provider_response = response
                .json::<Value>()
                .await
                .context("failed to decode AI response JSON")?;
            let chat_response: ChatCompletionResponse =
                serde_json::from_value(provider_response.clone())
                    .context("failed to decode chat completion response")?;
            let agent_response = match request.task {
                AgentTask::ResolveConflicts => {
                    AgentResponse::Patch(extract_resolve_conflicts_tool_args(&chat_response)?)
                }
                AgentTask::PlanCommit => {
                    AgentResponse::CommitPlan(extract_commit_plan_tool_args(&chat_response)?)
                }
            };
            Ok(ProviderExchange {
                provider_response,
                agent_response,
            })
        })
    }
}

/// Accumulated state for a single tool call being streamed.
#[derive(Default)]
struct StreamedToolCall {
    id: String,
    kind: String,
    name: String,
    arguments: String,
}

/// Parse an SSE stream and accumulate tool call arguments.
/// Emits heartbeat events to keep the user informed.
async fn parse_sse_tool_call(
    response: reqwest::Response,
    emitter: &mut Emitter,
) -> Result<Vec<StreamedToolCall>> {
    use futures_util::TryStreamExt;
    use tokio::io::AsyncBufReadExt;
    use tokio_util::io::StreamReader;

    let stream = response
        .bytes_stream()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));
    let reader = StreamReader::new(stream);
    let mut lines = reader.lines();

    let mut tool_calls: Vec<StreamedToolCall> = Vec::new();
    let mut chunk_count = 0u64;

    while let Some(line) = lines.next_line().await? {
        let line = line.trim().to_string();

        if line.is_empty() || line.starts_with(':') {
            continue;
        }

        let data = match line.strip_prefix("data: ") {
            Some(d) => d.trim(),
            None => continue,
        };

        if data == "[DONE]" {
            break;
        }

        let chunk: Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Extract delta tool_calls from chunk
        if let Some(choices) = chunk.get("choices").and_then(Value::as_array) {
            for choice in choices {
                let delta = match choice.get("delta") {
                    Some(d) => d,
                    None => continue,
                };

                if let Some(tc_array) = delta.get("tool_calls").and_then(Value::as_array) {
                    for tc in tc_array {
                        let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;

                        // Ensure we have enough slots
                        while tool_calls.len() <= index {
                            tool_calls.push(StreamedToolCall::default());
                        }

                        let entry = &mut tool_calls[index];

                        if let Some(id) = tc.get("id").and_then(Value::as_str) {
                            entry.id = id.to_string();
                        }
                        if let Some(kind) = tc.get("type").and_then(Value::as_str) {
                            entry.kind = kind.to_string();
                        }
                        if let Some(func) = tc.get("function") {
                            if let Some(name) = func.get("name").and_then(Value::as_str) {
                                entry.name = name.to_string();
                            }
                            if let Some(args) = func.get("arguments").and_then(Value::as_str) {
                                entry.arguments.push_str(args);
                            }
                        }
                    }
                }
            }
        }

        chunk_count += 1;
        // Emit periodic heartbeat so the user sees progress
        if chunk_count % 20 == 0 {
            let _ = emitter
                .emit(
                    "ai_response_delta",
                    Some("ai_wait"),
                    Some(".".to_string()),
                    None,
                )
                .await;
        }
    }

    if tool_calls.is_empty() {
        return Err(anyhow!(
            "SSE stream completed without any tool calls for commit planning"
        ));
    }

    Ok(tool_calls)
}

/// Convert accumulated streamed tool calls into a ChatCompletionResponse-shaped Value.
fn assembled_to_response(tool_calls: &[StreamedToolCall]) -> Value {
    let tc_values: Vec<Value> = tool_calls
        .iter()
        .map(|tc| {
            serde_json::json!({
                "id": tc.id,
                "type": if tc.kind.is_empty() { "function" } else { &tc.kind },
                "function": {
                    "name": tc.name,
                    "arguments": tc.arguments,
                }
            })
        })
        .collect();

    serde_json::json!({
        "choices": [{
            "message": {
                "tool_calls": tc_values
            }
        }]
    })
}
