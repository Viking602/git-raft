use crate::commit::{CommitPlan, CommitPlanningInputs};
use crate::config::CommitConfig;
use crate::events::Emitter;
use crate::git::{DiffStat, GitExec, GitSnapshot, RepoContext};
use crate::store::RunStore;
use anyhow::{Context, Result, anyhow};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::env;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AiClient {
    config: AiConfig,
    provider: OpenAiCompatProvider,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiConfig {
    pub base_url: String,
    pub model: String,
    #[serde(skip_serializing)]
    pub api_key: String,
    pub api_key_env: String,
    pub commit_format: String,
    pub commit_language: String,
    pub commit_use_gitmoji: bool,
    pub commit_include_body: bool,
    pub commit_include_footer: bool,
    pub commit_ignore_paths: Vec<String>,
    pub commit_examples_file: String,
    #[serde(skip_serializing)]
    pub commit_examples: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiPatch {
    pub confidence: f32,
    pub summary: String,
    pub files: Vec<ResolvedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedFile {
    pub path: String,
    pub explanation: String,
    pub resolved_content: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AgentTask {
    Ask,
    ResolveConflicts,
    PlanCommit,
}

impl AgentTask {
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::ResolveConflicts => "resolve_conflicts",
            Self::PlanCommit => "plan_commit",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RepoContextPayload {
    pub cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub git_snapshot: GitSnapshot,
    pub diff_stats: Vec<DiffStat>,
    pub recent_subjects: Vec<String>,
    pub commit_format: String,
    pub commit_language: String,
    pub commit_use_gitmoji: bool,
    pub commit_include_body: bool,
    pub commit_include_footer: bool,
    pub commit_ignore_paths: Vec<String>,
    pub commit_examples_file: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub commit_examples: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AgentRequest {
    pub task: AgentTask,
    pub system_prompt: String,
    pub user_payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_context: Option<RepoContextPayload>,
}

impl AgentRequest {
    pub(crate) fn task_name(&self) -> &'static str {
        self.task.as_str()
    }

    pub(crate) fn summary(&self) -> Value {
        let prompt_preview = self
            .user_payload
            .get("prompt")
            .and_then(Value::as_str)
            .map(|prompt| truncate_text(prompt, 160));
        let conflict_files = self
            .user_payload
            .get("conflicts")
            .and_then(Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(|file| file.get("path").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        json!({
            "task": self.task_name(),
            "hasRepoContext": self.repo_context.is_some(),
            "branch": self.repo_context.as_ref().and_then(|ctx| ctx.branch.clone()),
            "promptPreview": prompt_preview,
            "conflictFiles": conflict_files,
            "recentSubjectCount": self.repo_context.as_ref().map_or(0, |ctx| ctx.recent_subjects.len()),
            "diffStatCount": self.repo_context.as_ref().map_or(0, |ctx| ctx.diff_stats.len()),
        })
    }

    fn provider_request(&self, model: &str) -> Result<Value> {
        Ok(json!({
            "model": model,
            "messages": [
                {
                    "role": "system",
                    "content": self.system_prompt,
                },
                {
                    "role": "user",
                    "content": serde_json::to_string_pretty(&json!({
                        "task": self.task_name(),
                        "user_payload": self.user_payload,
                        "repo_context": self.repo_context,
                    }))?,
                }
            ]
        }))
    }
}

#[derive(Debug, Clone)]
enum AgentResponse {
    Text(String),
    Patch(AiPatch),
    CommitPlan(CommitPlan),
}

#[derive(Debug, Clone)]
struct ProviderExchange {
    provider_response: Value,
    agent_response: AgentResponse,
}

pub(crate) struct AiExchange {
    task: AgentTask,
    request: AgentRequest,
    provider_response: Value,
    response: AgentResponse,
}

impl AiExchange {
    pub(crate) fn task_name(&self) -> &'static str {
        self.task.as_str()
    }

    pub(crate) fn request_summary(&self) -> Value {
        self.request.summary()
    }

    pub(crate) fn response_summary(&self) -> Value {
        match &self.response {
            AgentResponse::Text(text) => json!({
                "task": self.task_name(),
                "kind": "text",
                "textPreview": truncate_text(text, 160),
            }),
            AgentResponse::Patch(patch) => json!({
                "task": self.task_name(),
                "kind": "patch",
                "summary": truncate_text(&patch.summary, 160),
                "patchConfidence": patch.confidence,
                "fileCount": patch.files.len(),
                "files": patch.files.iter().map(|file| file.path.clone()).collect::<Vec<_>>(),
            }),
            AgentResponse::CommitPlan(plan) => json!({
                "task": self.task_name(),
                "kind": "commit_plan",
                "groupCount": plan.groups.len(),
                "confidence": plan.confidence,
                "autoExecutable": plan.auto_executable,
                "warnings": plan.warnings,
            }),
        }
    }

    pub(crate) fn patch_confidence(&self) -> Option<f32> {
        match &self.response {
            AgentResponse::Patch(patch) => Some(patch.confidence),
            AgentResponse::CommitPlan(plan) => Some(plan.confidence),
            AgentResponse::Text(_) => None,
        }
    }

    pub(crate) fn response_record(&self) -> Value {
        json!({
            "task": self.task_name(),
            "response": match &self.response {
                AgentResponse::Text(text) => json!({
                    "kind": "text",
                    "text": text,
                }),
                AgentResponse::Patch(patch) => json!({
                    "kind": "patch",
                    "patch": patch,
                }),
                AgentResponse::CommitPlan(plan) => json!({
                    "kind": "commit_plan",
                    "commit_plan": plan,
                }),
            },
            "response_summary": self.response_summary(),
            "provider_response": &self.provider_response,
        })
    }

    pub(crate) fn into_text(self) -> Result<String> {
        match self.response {
            AgentResponse::Text(text) => Ok(text),
            AgentResponse::Patch(_) | AgentResponse::CommitPlan(_) => {
                Err(anyhow!("expected text AI response"))
            }
        }
    }

    pub(crate) fn into_patch(self) -> Result<AiPatch> {
        match self.response {
            AgentResponse::Patch(patch) => Ok(patch),
            AgentResponse::Text(_) | AgentResponse::CommitPlan(_) => {
                Err(anyhow!("expected patch AI response"))
            }
        }
    }

    pub(crate) fn into_commit_plan(self) -> Result<CommitPlan> {
        match self.response {
            AgentResponse::CommitPlan(plan) => Ok(plan),
            AgentResponse::Text(_) | AgentResponse::Patch(_) => {
                Err(anyhow!("expected commit plan AI response"))
            }
        }
    }
}

trait AiProvider {
    fn execute<'a>(
        &'a self,
        config: &'a AiConfig,
        request: &'a AgentRequest,
        provider_request: Value,
        emitter: &'a mut Emitter,
    ) -> Pin<Box<dyn Future<Output = Result<ProviderExchange>> + Send + 'a>>;
}

#[derive(Debug, Clone)]
struct OpenAiCompatProvider {
    http: Client,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: String,
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
            let mut request_body = provider_request;
            if matches!(request.task, AgentTask::PlanCommit)
                && let Some(object) = request_body.as_object_mut()
            {
                object.insert("stream".to_string(), Value::Bool(true));
            }
            let response = self
                .http
                .post(url)
                .bearer_auth(resolve_api_key(config)?)
                .json(&request_body)
                .send()
                .await?;
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            let (provider_response, content) = if content_type.contains("text/event-stream")
                && matches!(request.task, AgentTask::PlanCommit)
            {
                stream_chat_completion(response, request, emitter).await?
            } else {
                let provider_response = response
                    .json::<Value>()
                    .await
                    .context("failed to decode AI response JSON")?;
                let response: ChatCompletionResponse =
                    serde_json::from_value(provider_response.clone())
                        .context("failed to decode chat completion response")?;
                let content = response
                    .choices
                    .first()
                    .map(|choice| choice.message.content.clone())
                    .ok_or_else(|| anyhow!("empty AI response"))?;
                (provider_response, content)
            };
            let agent_response = match request.task {
                AgentTask::Ask => AgentResponse::Text(content),
                AgentTask::ResolveConflicts => AgentResponse::Patch(
                    serde_json::from_str(&content)
                        .context("AI response was not valid patch JSON")?,
                ),
                AgentTask::PlanCommit => AgentResponse::CommitPlan(
                    serde_json::from_str(&content)
                        .context("AI response was not valid commit plan JSON")?,
                ),
            };
            Ok(ProviderExchange {
                provider_response,
                agent_response,
            })
        })
    }
}

impl AiClient {
    pub fn from_repo(repo: Option<&RepoContext>) -> Result<Self> {
        let config = Self::config_from_repo(repo)?;
        if config.base_url.trim().is_empty() {
            return Err(anyhow!("missing GIT_RAFT_BASE_URL or [provider].base_url"));
        }
        let _api_key = resolve_api_key(&config)?;
        let http = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .context("failed to build http client")?;
        Ok(Self {
            config,
            provider: OpenAiCompatProvider { http },
        })
    }

    pub fn config_from_repo(repo: Option<&RepoContext>) -> Result<AiConfig> {
        let (repo_config, _) =
            crate::config::resolve_config(repo.map(|repo| repo.root_dir.as_path()))?;

        let base_url = env::var("GIT_RAFT_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| non_empty(&repo_config.provider.base_url))
            .unwrap_or_default();

        let api_key_env = non_empty(&repo_config.provider.api_key_env)
            .unwrap_or_else(|| "GIT_RAFT_API_KEY".to_string());
        let api_key = non_empty(&repo_config.provider.api_key).unwrap_or_default();

        let model = env::var("GIT_RAFT_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| non_empty(&repo_config.provider.model))
            .unwrap_or_else(|| "gpt-4.1-mini".to_string());

        let commit_format =
            non_empty(&repo_config.commit.format).unwrap_or_else(|| "conventional".to_string());
        let commit_language =
            non_empty(&repo_config.commit.language).unwrap_or_else(|| "en".to_string());
        let commit_use_gitmoji = repo_config.commit.use_gitmoji;
        let commit_include_body = repo_config.commit.include_body;
        let commit_include_footer = repo_config.commit.include_footer;
        let commit_ignore_paths = repo_config.commit.ignore_paths.clone();
        let commit_examples_file = non_empty(&repo_config.commit.examples_file)
            .unwrap_or_else(|| ".config/git-raft/commit_examples.md".to_string());
        let commit_examples = repo
            .and_then(|repo| {
                std::fs::read_to_string(repo.root_dir.join(&commit_examples_file)).ok()
            })
            .unwrap_or_default();

        Ok(AiConfig {
            base_url,
            model,
            api_key,
            api_key_env,
            commit_format,
            commit_language,
            commit_use_gitmoji,
            commit_include_body,
            commit_include_footer,
            commit_ignore_paths,
            commit_examples_file,
            commit_examples,
        })
    }

    pub(crate) const fn config(&self) -> &AiConfig {
        &self.config
    }

    pub(crate) fn build_ask_request(
        &self,
        prompt: String,
        repo_context: Option<RepoContextPayload>,
    ) -> AgentRequest {
        AgentRequest {
            task: AgentTask::Ask,
            system_prompt: self.ask_system_prompt(),
            user_payload: json!({ "prompt": prompt }),
            repo_context,
        }
    }

    pub(crate) async fn build_conflict_request(
        &self,
        git: &GitExec,
        conflicts: &[String],
        repo_context: Option<RepoContextPayload>,
    ) -> Result<AgentRequest> {
        let mut files = Vec::new();
        for path in conflicts {
            files.push(json!({
                "path": path,
                "base": git.read_stage_file(1, path).await.unwrap_or_default(),
                "ours": git.read_stage_file(2, path).await.unwrap_or_default(),
                "theirs": git.read_stage_file(3, path).await.unwrap_or_default(),
                "current": git.read_worktree_file(path).await.unwrap_or_default(),
            }));
        }
        Ok(AgentRequest {
            task: AgentTask::ResolveConflicts,
            system_prompt: "You resolve git merge conflicts. Reply with strict JSON: {\"confidence\":0.0-1.0,\"summary\":\"...\",\"files\":[{\"path\":\"...\",\"explanation\":\"...\",\"resolved_content\":\"...\"}]}. Only include files from the input.".to_string(),
            user_payload: json!({ "conflicts": files }),
            repo_context,
        })
    }

    pub(crate) fn build_commit_request(
        &self,
        planning_inputs: CommitPlanningInputs,
        intent: Option<String>,
        local_hint_plan: CommitPlan,
        commit_config: &CommitConfig,
        repo_context: Option<RepoContextPayload>,
    ) -> AgentRequest {
        AgentRequest {
            task: AgentTask::PlanCommit,
            system_prompt: self.commit_system_prompt(),
            user_payload: json!({
                "intent": intent,
                "changed_files": planning_inputs.changed_files,
                "staged_files": planning_inputs.staged_files,
                "unstaged_files": planning_inputs.unstaged_files,
                "untracked_files": planning_inputs.untracked_files,
                "format_preferences": {
                    "format": commit_config.format,
                    "language": commit_config.language,
                    "use_gitmoji": commit_config.use_gitmoji,
                    "include_body": commit_config.include_body,
                    "include_footer": commit_config.include_footer,
                },
                "ignore_paths": commit_config.ignore_paths,
                "local_hint_plan": local_hint_plan,
            }),
            repo_context,
        }
    }

    pub(crate) async fn execute(
        &self,
        request: AgentRequest,
        emitter: &mut Emitter,
        store: Option<&RunStore>,
    ) -> Result<AiExchange> {
        let provider_request = request.provider_request(&self.config.model)?;
        if let Some(store) = store {
            store.write_json(
                "ai-request.json",
                &json!({
                    "task": request.task_name(),
                    "request": &request,
                    "provider_request": &provider_request,
                }),
            )?;
        }
        emitter
            .emit(
                "ai_request_started",
                Some("ai_wait"),
                Some(format!(
                    "sending {} request to AI provider",
                    request.task_name()
                )),
                Some(json!({
                    "task": request.task_name(),
                    "model": self.config.model,
                })),
            )
            .await?;
        emitter
            .emit(
                "phase_changed",
                Some("ai_wait"),
                Some("waiting for AI provider".to_string()),
                Some(json!({
                    "task": request.task_name(),
                    "model": self.config.model,
                })),
            )
            .await?;

        match self
            .provider
            .execute(&self.config, &request, provider_request.clone(), emitter)
            .await
        {
            Ok(provider_exchange) => {
                let exchange = AiExchange {
                    task: request.task.clone(),
                    request,
                    provider_response: provider_exchange.provider_response,
                    response: provider_exchange.agent_response,
                };
                if let Some(store) = store {
                    store.write_json("ai-response.json", &exchange.response_record())?;
                }
                emitter
                    .emit(
                        "ai_response_ready",
                        Some("ai_wait"),
                        Some(format!(
                            "received {} response from AI provider",
                            exchange.task_name()
                        )),
                        Some(exchange.response_summary()),
                    )
                    .await?;
                Ok(exchange)
            }
            Err(err) => {
                let message = err.to_string();
                if message.contains("decode")
                    || message.contains("empty AI response")
                    || message.contains("valid patch JSON")
                    || message.contains("valid commit plan JSON")
                {
                    emitter
                        .emit(
                            "ai_response_invalid",
                            Some("ai_wait"),
                            Some(message.clone()),
                            Some(json!({
                                "task": request.task_name(),
                                "model": self.config.model,
                            })),
                        )
                        .await?;
                }
                Err(err)
            }
        }
    }

    fn ask_system_prompt(&self) -> String {
        let mut prompt =
            "You are a concise git workflow assistant. Reply with direct technical guidance."
                .to_string();
        prompt.push_str(
            "\nIf the user asks for a commit message, follow the configured commit format preset: ",
        );
        prompt.push_str(&self.config.commit_format);
        prompt.push_str("\nDefault commit subject language: ");
        prompt.push_str(match self.config.commit_language.as_str() {
            "zh" => "Chinese",
            _ => "English",
        });
        prompt.push_str(
            "\nSupported presets in this repository are conventional, angular, gitmoji, and simple.",
        );
        if !self.config.commit_examples.is_empty() {
            prompt.push_str("\nUse these repository commit examples when they help:\n");
            prompt.push_str(&self.config.commit_examples);
        }
        prompt
    }

    fn commit_system_prompt(&self) -> String {
        let mut prompt = "You build git commit plans. Reply with strict JSON only: {\"groups\":[{\"scope\":string|null,\"files\":[string],\"commit_message\":string,\"rationale\":string}],\"confidence\":0.0-1.0,\"warnings\":[string],\"auto_executable\":boolean}.".to_string();
        prompt.push_str("\nRules:");
        prompt.push_str("\n- Include every changed file at most once.");
        prompt
            .push_str("\n- Build commit groups and commit messages from the provided preferences.");
        prompt.push_str("\n- Keep rationale short and specific.");
        prompt.push_str("\n- Lower confidence if grouping is ambiguous.");
        if !self.config.commit_examples.is_empty() {
            prompt.push_str("\nRepository commit examples:\n");
            prompt.push_str(&self.config.commit_examples);
        }
        prompt
    }
}

async fn stream_chat_completion(
    response: reqwest::Response,
    request: &AgentRequest,
    emitter: &mut Emitter,
) -> Result<(Value, String)> {
    let mut stream = response.bytes_stream();
    let mut pending = String::new();
    let mut collected = String::new();
    let mut chunks = Vec::new();

    while let Some(item) = stream.next().await {
        let bytes = item.context("failed to read AI stream chunk")?;
        pending.push_str(&String::from_utf8_lossy(&bytes));

        while let Some(index) = pending.find("\n\n") {
            let frame = pending[..index].to_string();
            pending.drain(..index + 2);

            for line in frame.lines() {
                let Some(data) = line.strip_prefix("data: ") else {
                    continue;
                };
                let data = data.trim();
                if data.is_empty() {
                    continue;
                }
                if data == "[DONE]" {
                    return Ok((
                        json!({
                            "kind": "stream",
                            "task": request.task_name(),
                            "chunks": chunks,
                        }),
                        collected,
                    ));
                }

                let value: Value =
                    serde_json::from_str(data).context("failed to decode AI stream JSON chunk")?;
                if let Some(delta) = extract_stream_delta(&value) {
                    collected.push_str(&delta);
                    emitter
                        .emit(
                            "ai_response_delta",
                            Some("ai_wait"),
                            Some(delta.clone()),
                            Some(json!({ "task": request.task_name() })),
                        )
                        .await?;
                    chunks.push(delta);
                }
            }
        }
    }

    Ok((
        json!({
            "kind": "stream",
            "task": request.task_name(),
            "chunks": chunks,
        }),
        collected,
    ))
}

fn extract_stream_delta(value: &Value) -> Option<String> {
    let delta = value.get("choices")?.as_array()?.first()?.get("delta")?;
    delta
        .get("content")
        .and_then(Value::as_str)
        .or_else(|| delta.get("reasoning").and_then(Value::as_str))
        .or_else(|| delta.get("reasoning_content").and_then(Value::as_str))
        .map(str::to_string)
}

pub(crate) async fn collect_repo_context(
    git: &GitExec,
    repo: &RepoContext,
    config: &AiConfig,
) -> RepoContextPayload {
    let git_snapshot = git.inspect_snapshot().await.unwrap_or_default();
    RepoContextPayload {
        cwd: repo.root_dir.display().to_string(),
        branch: git_snapshot.branch.clone(),
        diff_stats: git_snapshot.diff_stats.clone(),
        recent_subjects: git.recent_subjects(20).await.unwrap_or_default(),
        git_snapshot,
        commit_format: config.commit_format.clone(),
        commit_language: config.commit_language.clone(),
        commit_use_gitmoji: config.commit_use_gitmoji,
        commit_include_body: config.commit_include_body,
        commit_include_footer: config.commit_include_footer,
        commit_ignore_paths: config.commit_ignore_paths.clone(),
        commit_examples_file: config.commit_examples_file.clone(),
        commit_examples: config.commit_examples.clone(),
    }
}

fn truncate_text(text: &str, limit: usize) -> String {
    let truncated = text.trim();
    let mut chars = truncated.chars();
    let preview = chars.by_ref().take(limit).collect::<String>();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn resolve_api_key(config: &AiConfig) -> Result<String> {
    if !config.api_key.trim().is_empty() {
        return Ok(config.api_key.clone());
    }
    env::var(&config.api_key_env).with_context(|| {
        format!(
            "missing provider.api_key or {} for AI calls",
            config.api_key_env
        )
    })
}
