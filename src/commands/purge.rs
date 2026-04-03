use crate::events::Emitter;
use crate::git::{self, GitExec};
use anyhow::{Result, anyhow};
use serde_json::json;
use std::path::PathBuf;

pub(crate) struct PurgeRun {
    pub(crate) paths: Vec<String>,
    pub(crate) force: bool,
    pub(crate) push: bool,
}

pub(crate) async fn run_purge(
    request: PurgeRun,
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    emitter: &mut Emitter,
) -> Result<()> {
    if request.push && !request.force {
        return Err(anyhow!("--push requires --force"));
    }
    if request.paths.is_empty() {
        return Err(anyhow!("no paths specified"));
    }

    let repo_ctx = repo.ok_or_else(|| anyhow!("purge requires a git repository"))?;
    let git = GitExec::new(cwd.clone(), Some(repo_ctx.clone()));

    emitter
        .emit(
            "phase_changed",
            Some("scan"),
            Some("checking paths in history".to_string()),
            None,
        )
        .await?;

    let mut found_paths = Vec::new();
    for path in &request.paths {
        if git.path_exists_in_history(path).await? {
            found_paths.push(path.clone());
        } else {
            emitter
                .emit(
                    "warning",
                    Some("scan"),
                    Some(format!("path not found in history: {path}")),
                    None,
                )
                .await?;
        }
    }

    if found_paths.is_empty() {
        emitter
            .emit(
                "tool_result",
                Some("done"),
                Some("purge_noop".to_string()),
                Some(json!({ "paths": request.paths, "removed": 0 })),
            )
            .await?;
        if !emitter.json_mode() {
            println!("No matching paths found in history. Nothing to do.");
        }
        return Ok(());
    }

    // Check if affected commits have been pushed
    if !request.force {
        let has_pushed = git.has_pushed_commits().await?;
        if has_pushed {
            let msg = format!(
                "{} paths to purge, but branch has pushed commits.\n  \
                 Rerun with --force to rewrite, or --force --push to rewrite and push.",
                found_paths.len()
            );
            emitter
                .emit(
                    "commandFailed",
                    Some("exec"),
                    Some(msg.clone()),
                    Some(json!({ "paths": found_paths })),
                )
                .await?;
            if !emitter.json_mode() {
                println!("\u{2717} {msg}");
            }
            return Err(anyhow!("commits already pushed; use --force"));
        }
    }

    emitter
        .emit(
            "phase_changed",
            Some("exec"),
            Some(format!("purging {} paths from history", found_paths.len())),
            Some(json!({ "paths": &found_paths })),
        )
        .await?;

    git.purge_paths(&found_paths).await?;

    // Add purged paths to .gitignore
    let gitignore_path = cwd.join(".gitignore");
    let existing = if gitignore_path.exists() {
        tokio::fs::read_to_string(&gitignore_path)
            .await
            .unwrap_or_default()
    } else {
        String::new()
    };
    let mut new_entries = Vec::new();
    for path in &found_paths {
        let rule = normalize_gitignore_entry(path);
        if !existing.lines().any(|line| line.trim() == rule) {
            new_entries.push(rule);
        }
    }
    if !new_entries.is_empty() {
        let mut content = existing;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        for entry in &new_entries {
            content.push_str(entry);
            content.push('\n');
        }
        tokio::fs::write(&gitignore_path, &content).await?;
        git.stage_files(&[".gitignore".to_string()]).await?;
        git.create_commit(&format!(
            "chore: add purged paths to .gitignore\n\n{}",
            new_entries
                .iter()
                .map(|e| format!("- {e}"))
                .collect::<Vec<_>>()
                .join("\n")
        ))
        .await?;
    }

    if request.push {
        emitter
            .emit(
                "phase_changed",
                Some("exec"),
                Some("force pushing to remote".to_string()),
                None,
            )
            .await?;
        git.force_push_with_lease().await?;
    }

    emitter
        .emit(
            "tool_result",
            Some("done"),
            Some("purge".to_string()),
            Some(json!({
                "paths": found_paths,
                "removed": found_paths.len(),
                "pushed": request.push,
                "gitignore_added": new_entries,
            })),
        )
        .await?;
    if !emitter.json_mode() {
        println!("\u{2713} Purged {} paths from history:", found_paths.len());
        for path in &found_paths {
            println!("  - {path}");
        }
        if !new_entries.is_empty() {
            println!("  Added to .gitignore and committed.");
        }
        if request.push {
            println!("  Force pushed to remote.");
        }
    }

    Ok(())
}

fn normalize_gitignore_entry(path: &str) -> String {
    let trimmed = path.trim().trim_end_matches('/');
    format!("/{trimmed}/")
}
