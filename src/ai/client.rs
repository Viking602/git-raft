use crate::commit::CommitPlanningInputs;
use crate::config::CommitConfig;
use crate::commands::merge_rebase::retention::{ConflictTextFile, preservation_requirements};
use crate::events::Emitter;
use crate::git::{GitExec, RepoContext};
use crate::store::RunStore;
use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde_json::Value;
use serde_json::json;
use std::time::Duration;

use super::AiClient;
use super::config::{config_from_repo, resolve_api_key};
use super::exchange::AiExchange;
use super::provider::AiProvider;
use super::request::{AgentRequest, AgentTask, RepoContextPayload};

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
            provider: super::provider::OpenAiCompatProvider { http },
        })
    }

    pub fn config_from_repo(repo: Option<&RepoContext>) -> Result<super::AiConfig> {
        config_from_repo(repo)
    }

    pub(crate) const fn config(&self) -> &super::AiConfig {
        &self.config
    }

    pub(crate) async fn build_conflict_request(
        &self,
        git: &GitExec,
        conflicts: &[String],
        repo_context: Option<RepoContextPayload>,
        attempt: usize,
        repair_context: Option<Value>,
    ) -> Result<AgentRequest> {
        let mut files = Vec::new();
        let mut conflict_files = Vec::new();
        for path in conflicts {
            let base = git.read_stage_file(1, path).await.unwrap_or_default();
            let ours = git.read_stage_file(2, path).await.unwrap_or_default();
            let theirs = git.read_stage_file(3, path).await.unwrap_or_default();
            let current = git.read_worktree_file(path).await.unwrap_or_default();
            files.push(json!({
                "path": path,
                "base": base,
                "ours": ours,
                "theirs": theirs,
                "current": current,
            }));
            conflict_files.push(ConflictTextFile {
                path: path.clone(),
                current,
            });
        }
        let requirements = preservation_requirements(&conflict_files)
            .map_err(|err| anyhow!("failed to build conflict preservation requirements: {err}"))?;
        Ok(AgentRequest {
            task: AgentTask::ResolveConflicts,
            system_prompt: concat!(
                "You resolve git merge conflicts. Reply with strict JSON: ",
                "{\"confidence\":0.0-1.0,\"summary\":\"...\",\"files\":[{\"path\":\"...\",\"explanation\":\"...\",\"resolved_content\":\"...\"}]}. ",
                "Only include files from the input. ",
                "Return complete file contents, not patches or snippets. ",
                "Keep every unique line from ours and theirs in the resolved file text unless that exact line is duplicated on both sides. ",
                "When you combine behavior, keep the original unique lines verbatim and add code around them. ",
                "Do not rewrite or paraphrase away unique lines. ",
                "The preservation_requirements field lists exact lines and multi-line blocks that must appear verbatim in resolved_content. ",
                "If two unique lines would conflict in the same expression or return path, preserve them by wrapping each side in a helper closure, local block, or helper variable so the original lines still appear verbatim. ",
                "Prefer preserving exact original statements over inventing renamed equivalents. ",
                "Match the repository formatter conventions exactly; for Rust code, return rustfmt-clean file contents. ",
                "Do not change function names or call expressions inside required test blocks; if needed, add wrapper functions in production code and keep the original test lines verbatim. ",
                "Do not delete non-duplicate code from either side of a conflict. ",
                "You may deduplicate repeated lines, but you must not omit unique logic, comments, or behavior from ours or theirs. ",
                "Preserve all non-duplicate code and keep the result buildable."
            )
            .to_string(),
            user_payload: json!({
                "attempt": attempt,
                "conflicts": files,
                "preservation_requirements": requirements,
                "repair_context": repair_context,
            }),
            repo_context,
        })
    }

    pub(crate) fn build_commit_request(
        &self,
        planning_inputs: CommitPlanningInputs,
        intent: Option<String>,
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
                "split_requirements": {
                    "independent_commits": true,
                    "description": "Only split commits when each resulting commit can be pulled independently and still run correctly on its own."
                },
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
                let exchange = AiExchange::new(
                    request.task.clone(),
                    request,
                    provider_exchange.provider_response,
                    provider_exchange.agent_response,
                );
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
                    || message.contains("resolve_conflicts tool call")
                    || message.contains("valid resolve_conflicts tool arguments")
                    || message.contains("valid commit plan JSON")
                    || message.contains("plan_commit tool call")
                    || message.contains("commit plan tool arguments")
                    || message.contains("tool calls for commit planning")
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

    fn commit_system_prompt(&self) -> String {
        let mut prompt =
            "You build git commit plans. Use the provided `plan_commit` tool to return the result. Do not answer with plain text."
                .to_string();
        prompt.push_str("\nRules:");
        prompt.push_str("\n- Include every changed file at most once.");
        prompt
            .push_str("\n- Build commit groups and commit messages from the provided preferences.");
        prompt
            .push_str("\n- Decide whether the changes should be a single commit or split commits.");
        prompt.push_str("\n- Only use grouping_decision=\"split\" when grouping_confidence is high and the boundaries are reliable.");
        prompt.push_str("\n- If you split commits, each resulting commit must remain runnable on its own when pulled independently.");
        prompt.push_str("\n- Do not separate changes into different groups when one group depends on uncommitted changes from another group.");
        prompt.push_str("\n- If you cannot keep every split commit independently runnable, return a single commit plan instead.");
        prompt.push_str("\n- Always return single_group as a valid one-commit fallback for the entire change set.");
        prompt.push_str("\n- Keep rationale short and specific.");
        prompt.push_str("\n- Lower confidence if grouping is ambiguous.");
        if !self.config.commit_examples.is_empty() {
            prompt.push_str("\nRepository commit examples:\n");
            prompt.push_str(&self.config.commit_examples);
        }
        prompt
    }
}
