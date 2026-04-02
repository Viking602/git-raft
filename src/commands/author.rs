use crate::events::Emitter;
use crate::git::{self, GitExec};
use anyhow::{Result, anyhow};
use serde_json::json;
use std::path::PathBuf;

const AUTHOR_REWRITE_LIMIT: usize = 500;

#[derive(Clone, Debug, Eq, PartialEq)]
struct AuthorIdentity {
    name: String,
    email: String,
}

impl AuthorIdentity {
    fn new(name: String, email: String) -> Self {
        Self { name, email }
    }

    fn matches(&self, name: &str, email: &str) -> bool {
        self.name == name && self.email == email
    }

    fn display(&self) -> String {
        format!("{} <{}>", self.name, self.email)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RewritePlan {
    count: usize,
    use_root: bool,
}

pub(crate) struct AuthorRun {
    pub(crate) name: String,
    pub(crate) email: String,
    pub(crate) force: bool,
    pub(crate) push: bool,
}

pub(crate) async fn run_author(
    request: AuthorRun,
    cwd: PathBuf,
    repo: Option<git::RepoContext>,
    emitter: &mut Emitter,
) -> Result<()> {
    if request.push && !request.force {
        return Err(anyhow!("--push requires --force"));
    }

    let repo_ctx = repo.ok_or_else(|| anyhow!("author requires a git repository"))?;
    let git = GitExec::new(cwd.clone(), Some(repo_ctx.clone()));
    let target_author = AuthorIdentity::new(request.name, request.email);
    let current_author = git
        .preferred_user()
        .await?
        .map(|(name, email)| AuthorIdentity::new(name, email));
    let log = git.log_authors(AUTHOR_REWRITE_LIMIT + 1).await?;
    let rewrite = plan_rewrite(
        &log,
        AUTHOR_REWRITE_LIMIT,
        current_author.as_ref(),
        &target_author,
    );

    if rewrite.count == 0 {
        git.set_local_user(&target_author.name, &target_author.email)
            .await?;
        emitter
            .emit(
                "tool_result",
                Some("done"),
                Some("author_set".to_string()),
                Some(json!({
                    "name": target_author.name,
                    "email": target_author.email,
                    "rewritten": 0,
                })),
            )
            .await?;
        if !emitter.json_mode() {
            println!(
                "\u{2713} Project author set: {} <{}>\n  No commits need rewriting.",
                target_author.name, target_author.email
            );
        }
        return Ok(());
    }

    let hashes_to_rewrite: Vec<&str> = log[..rewrite.count.min(log.len())]
        .iter()
        .map(|(hash, _, _)| hash.as_str())
        .collect();
    let mut pushed_count = 0usize;
    for hash in &hashes_to_rewrite {
        if git.is_commit_pushed(hash).await? {
            pushed_count += 1;
        }
    }

    if pushed_count > 0 && !request.force {
        let msg = format!(
            "{} commits need rewriting, but {pushed_count} are already pushed to remote.\n  \
             Rerun with --force to rewrite, or --force --push to rewrite and push.",
            rewrite.count
        );
        emitter
            .emit(
                "commandFailed",
                Some("exec"),
                Some(msg.clone()),
                Some(json!({
                    "rewrite_count": rewrite.count,
                    "pushed_count": pushed_count,
                })),
            )
            .await?;
        if !emitter.json_mode() {
            println!("\u{2717} {msg}");
        }
        return Err(anyhow!("commits already pushed; use --force"));
    }

    emitter
        .emit(
            "phase_changed",
            Some("exec"),
            Some(format!("rewriting {} commits", rewrite.count)),
            None,
        )
        .await?;

    git.rewrite_author(
        rewrite.count,
        &target_author.name,
        &target_author.email,
        rewrite.use_root,
    )
    .await?;
    git.set_local_user(&target_author.name, &target_author.email)
        .await?;

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

    let old_author = current_author
        .as_ref()
        .map(AuthorIdentity::display)
        .unwrap_or_else(|| "unknown".to_string());
    let new_author = target_author.display();
    emitter
        .emit(
            "tool_result",
            Some("done"),
            Some("author_rewrite".to_string()),
            Some(json!({
                "name": target_author.name,
                "email": target_author.email,
                "rewritten": rewrite.count,
                "pushed": request.push,
            })),
        )
        .await?;
    if !emitter.json_mode() {
        println!(
            "\u{2713} Project author set: {new_author}\n  \
             Rewrote {rewrite_count} commits (HEAD~{}..HEAD)\n  \
             old: {old_author}\n  \
             new: {new_author}",
            rewrite.count - 1,
            rewrite_count = rewrite.count
        );
        if request.push {
            println!("  Force pushed to remote.");
        }
    }

    Ok(())
}

fn plan_rewrite(
    log: &[(String, String, String)],
    limit: usize,
    current_author: Option<&AuthorIdentity>,
    target_author: &AuthorIdentity,
) -> RewritePlan {
    let Some(current_author) = current_author else {
        return RewritePlan {
            count: 0,
            use_root: false,
        };
    };
    if current_author == target_author {
        return RewritePlan {
            count: 0,
            use_root: false,
        };
    }

    let window_len = log.len().min(limit);
    let mut count = 0usize;
    for (_, name, email) in log.iter().take(window_len) {
        if current_author.matches(name, email) {
            count += 1;
        } else {
            break;
        }
    }

    RewritePlan {
        count,
        use_root: count > 0 && count == window_len && log.len() <= limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn author(name: &str, email: &str) -> AuthorIdentity {
        AuthorIdentity::new(name.to_string(), email.to_string())
    }

    #[test]
    fn rewrite_plan_stays_inside_the_limit_window() {
        let mut log = Vec::new();
        for i in 0..501 {
            log.push((
                format!("hash-{i}"),
                "Test User".to_string(),
                "test@example.com".to_string(),
            ));
        }

        let plan = plan_rewrite(
            &log,
            500,
            Some(&author("Test User", "test@example.com")),
            &author("Viking", "viking@example.com"),
        );

        assert_eq!(plan.count, 500);
        assert!(!plan.use_root);
    }
}
