//! Repository health diagnostics.

use anyhow::Result;
use futures::stream::{FuturesUnordered, StreamExt};

use crate::core::{
    acquire_semaphore_permit, clean_error_message, create_processing_context, init_command,
    set_terminal_title, set_terminal_title_and_flush, GIT_CONCURRENT_CAP, NO_REPOS_MESSAGE,
};
use crate::git::operations::run_git;
use crate::git::remote::{remote_policy_violation, RemoteDirection};

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";

/// Diagnose common blockers without mutating repositories.
pub async fn handle_doctor_command() -> Result<()> {
    set_terminal_title("🩺 repos doctor");

    let (start_time, repos) = init_command(SCANNING_MESSAGE).await;
    if repos.is_empty() {
        println!("\r{NO_REPOS_MESSAGE}");
        set_terminal_title_and_flush("✅ repos doctor");
        return Ok(());
    }

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    print!("\r🩺 Diagnosing {total_repos} {repo_word}                    \n\n");

    let context =
        match create_processing_context(std::sync::Arc::new(repos), start_time, GIT_CONCURRENT_CAP)
        {
            Ok(context) => context,
            Err(e) => {
                set_terminal_title_and_flush("✅ repos doctor");
                return Err(e);
            }
        };

    let unhealthy_repos = run_diagnostics(context).await;
    set_terminal_title_and_flush("✅ repos doctor");
    if unhealthy_repos > 0 {
        anyhow::bail!("{unhealthy_repos} repositories need attention");
    }
    Ok(())
}

async fn run_diagnostics(context: crate::core::ProcessingContext) -> usize {
    let mut futures = FuturesUnordered::new();
    let max_name_length = context.max_name_length;

    for (repo_name, repo_path) in context.repositories.iter() {
        let semaphore = std::sync::Arc::clone(&context.semaphore);

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore).await;
            let (findings, advisories) = diagnose_repo(repo_path).await;
            let symbol = if !findings.is_empty() || !advisories.is_empty() {
                "🟡"
            } else {
                "🟢"
            };
            let message = if findings.is_empty() && advisories.is_empty() {
                "healthy".to_string()
            } else {
                findings
                    .iter()
                    .chain(&advisories)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            println!(
                "{symbol} {repo_name:width$}  {message}",
                width = max_name_length
            );
            !findings.is_empty()
        };

        futures.push(future);
    }

    let mut unhealthy_repos = 0;
    while let Some(unhealthy) = futures.next().await {
        unhealthy_repos += usize::from(unhealthy);
    }

    let mut nested_drift = false;
    if let Ok(statuses) = crate::subrepo::status::analyze_subrepos() {
        if statuses.iter().any(|status| status.has_drift) {
            nested_drift = true;
            crate::subrepo::status::display_drift_summary(&statuses);
        }
    }

    println!();
    unhealthy_repos + usize::from(nested_drift)
}

async fn diagnose_repo(path: &std::path::Path) -> (Vec<String>, Vec<String>) {
    let mut findings = Vec::new();
    let mut advisories = Vec::new();

    match run_git(path, &["rev-parse", "--abbrev-ref", "HEAD"]).await {
        Ok((true, branch, _)) if branch == "HEAD" => findings.push("detached HEAD".to_string()),
        Ok((true, _, _)) => {}
        Ok((false, _, stderr)) => findings.push(format!("branch check failed: {stderr}")),
        Err(e) => findings.push(format!("branch check failed: {e}")),
    }

    let remotes = match run_git(path, &["remote"]).await {
        Ok((true, remotes, _)) if remotes.trim().is_empty() => {
            findings.push("no remote".to_string());
            Vec::new()
        }
        Ok((true, remotes, _)) => remotes.lines().map(str::to_string).collect(),
        Ok((false, _, stderr)) => {
            findings.push(format!("remote check failed: {stderr}"));
            Vec::new()
        }
        Err(e) => {
            findings.push(format!("remote check failed: {e}"));
            Vec::new()
        }
    };

    for remote in remotes {
        let url_key = format!("remote.{remote}.url");
        if let Ok((true, url, _)) = run_git(path, &["config", "--get", &url_key]).await {
            if let Some(advisory) = transport_advisory(&remote, &url) {
                advisories.push(advisory);
            }
        }

        match remote_policy_violation(path, &remote, RemoteDirection::Fetch).await {
            Ok(Some(violation)) => {
                findings.push(violation.message());
                continue;
            }
            Ok(None) => {}
            Err(error) => {
                findings.push(format!("{remote} URL inspection failed: {error}"));
                continue;
            }
        }

        match run_git(path, &["ls-remote", "--heads", &remote]).await {
            Ok((true, _, _)) => {}
            Ok((false, _, stderr)) => findings.push(format!(
                "{remote} access failed: {}",
                clean_error_message(&stderr)
            )),
            Err(e) => findings.push(format!(
                "{remote} access failed: {}",
                clean_error_message(&e.to_string())
            )),
        }
    }

    match run_git(path, &["rev-parse", "--abbrev-ref", "@{upstream}"]).await {
        Ok((true, _, _)) => {}
        Ok((false, _, _)) => findings.push("no upstream".to_string()),
        Err(_) => findings.push("no upstream".to_string()),
    }

    match run_git(path, &["status", "--porcelain"]).await {
        Ok((true, status, _)) => {
            if status
                .lines()
                .any(|line| line.starts_with("UU") || line.starts_with("AA"))
            {
                findings.push("conflicts".to_string());
            } else if !status.trim().is_empty() {
                findings.push("dirty worktree".to_string());
            }
        }
        Ok((false, _, stderr)) => findings.push(format!("status failed: {stderr}")),
        Err(e) => findings.push(format!("status failed: {e}")),
    }

    (findings, advisories)
}

fn transport_advisory(remote: &str, url: &str) -> Option<String> {
    let url = url.trim().to_ascii_lowercase();
    (url.starts_with("https://") || url.starts_with("http://")).then(|| {
        format!(
            "warning: {remote} uses HTTP(S); SSH-only setup: git remote set-url {remote} <SSH clone URL>"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::transport_advisory;

    #[test]
    fn warns_for_http_remotes_without_echoing_the_url() {
        let url = "https://token@example.com/team/repo.git";
        let advisory = transport_advisory("origin", url).expect("HTTPS should produce a warning");

        assert!(advisory.contains("origin uses HTTP(S)"));
        assert!(!advisory.contains("token"));
        assert!(transport_advisory("origin", "git@example.com:team/repo.git").is_none());
        assert!(transport_advisory("origin", "ssh://git@example.com/team/repo.git").is_none());
        assert!(transport_advisory("origin", "/tmp/repo.git").is_none());
    }
}
