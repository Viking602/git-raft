use crate::config;
use crate::events::Emitter;
use crate::git::{GitExec, RepoContext};
use crate::store::RunStore;
use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::env;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AiClient {
    config: AiConfig,
    http: Client,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiConfig {
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
    pub commit_format: String,
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

impl AiClient {
    pub fn from_repo(repo: Option<&RepoContext>) -> Result<Self> {
        let config = Self::config_from_repo(repo)?;
        if config.base_url.trim().is_empty() {
            return Err(anyhow!("missing GIT_RAFT_BASE_URL or [provider].base_url"));
        }
        let _api_key = env::var(&config.api_key_env)
            .with_context(|| format!("missing {} for AI calls", config.api_key_env))?;
        let http = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .context("failed to build http client")?;
        Ok(Self { config, http })
    }

    pub fn config_from_repo(repo: Option<&RepoContext>) -> Result<AiConfig> {
        let (repo_config, _) = config::resolve_config(repo.map(|repo| repo.root_dir.as_path()))?;

        let base_url = env::var("GIT_RAFT_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| non_empty(&repo_config.provider.base_url))
            .unwrap_or_default();

        let api_key_env = non_empty(&repo_config.provider.api_key_env)
            .unwrap_or_else(|| "GIT_RAFT_API_KEY".to_string());

        let model = env::var("GIT_RAFT_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| non_empty(&repo_config.provider.model))
            .unwrap_or_else(|| "gpt-4.1-mini".to_string());

        let commit_format =
            non_empty(&repo_config.commit.format).unwrap_or_else(|| "conventional".to_string());
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
            api_key_env,
            commit_format,
            commit_examples_file,
            commit_examples,
        })
    }

    pub async fn ask(
        &self,
        prompt: &str,
        emitter: &mut Emitter,
        store: Option<&RunStore>,
    ) -> Result<String> {
        let body = json!({
            "model": self.config.model,
            "messages": [
                {
                    "role": "system",
                    "content": self.ask_system_prompt(),
                },
                {
                    "role": "user",
                    "content": prompt,
                }
            ]
        });
        let value = self.chat(body, emitter, store).await?;
        let response: ChatCompletionResponse = serde_json::from_value(value)?;
        response
            .choices
            .first()
            .map(|choice| choice.message.content.clone())
            .ok_or_else(|| anyhow!("empty AI response"))
    }

    pub async fn resolve_conflicts(
        &self,
        git: &GitExec,
        conflicts: &[String],
        emitter: &mut Emitter,
        store: Option<&RunStore>,
    ) -> Result<AiPatch> {
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
        let body = json!({
            "model": self.config.model,
            "messages": [
                {
                    "role": "system",
                    "content": "You resolve git merge conflicts. Reply with strict JSON: {\"confidence\":0.0-1.0,\"summary\":\"...\",\"files\":[{\"path\":\"...\",\"explanation\":\"...\",\"resolved_content\":\"...\"}]}. Only include files from the input."
                },
                {
                    "role": "user",
                    "content": format!("Resolve these git conflicts: {}", serde_json::to_string_pretty(&files)?)
                }
            ]
        });
        let value = self.chat(body, emitter, store).await?;
        let response: ChatCompletionResponse = serde_json::from_value(value)?;
        let content = response
            .choices
            .first()
            .map(|choice| choice.message.content.clone())
            .ok_or_else(|| anyhow!("empty AI response"))?;
        serde_json::from_str(&content).context("AI response was not valid patch JSON")
    }

    async fn chat(
        &self,
        body: serde_json::Value,
        emitter: &mut Emitter,
        store: Option<&RunStore>,
    ) -> Result<serde_json::Value> {
        if let Some(store) = store {
            store.write_json("ai-request.json", &body)?;
        }
        emitter
            .emit(
                "phase_changed",
                Some("ai_wait"),
                Some("waiting for AI provider".to_string()),
                Some(json!({ "model": self.config.model })),
            )
            .await?;

        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let request = self
            .http
            .post(url)
            .bearer_auth(env::var(&self.config.api_key_env)?)
            .json(&body)
            .send();
        tokio::pin!(request);
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        let response = loop {
            tokio::select! {
                result = &mut request => break result?,
                _ = interval.tick() => {
                    emitter.emit(
                        "heartbeat",
                        Some("ai_wait"),
                        Some("waiting for AI response".to_string()),
                        None,
                    ).await?;
                }
            }
        };
        let value = response
            .json::<serde_json::Value>()
            .await
            .context("failed to decode AI response JSON")?;
        if let Some(store) = store {
            store.write_json("ai-response.json", &value)?;
        }
        Ok(value)
    }

    fn ask_system_prompt(&self) -> String {
        let mut prompt =
            "You are a concise git workflow assistant. Reply with direct technical guidance."
                .to_string();
        prompt.push_str(
            "\nIf the user asks for a commit message, follow the configured commit format preset: ",
        );
        prompt.push_str(&self.config.commit_format);
        prompt.push_str("\nSupported presets in this repository are conventional, angular, gitmoji, and simple.");
        if !self.config.commit_examples.is_empty() {
            prompt.push_str("\nUse these repository commit examples when they help:\n");
            prompt.push_str(&self.config.commit_examples);
        }
        prompt
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
