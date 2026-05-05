//! Repository health diagnostics.

use anyhow::Result;
use futures::stream::{FuturesUnordered, StreamExt};

use crate::core::{
    acquire_semaphore_permit, create_processing_context, init_command, set_terminal_title,
    set_terminal_title_and_flush, GIT_CONCURRENT_CAP, NO_REPOS_MESSAGE,
};
use crate::git::operations::run_git;

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

    run_diagnostics(context).await;
    set_terminal_title_and_flush("✅ repos doctor");
    Ok(())
}

async fn run_diagnostics(context: crate::core::ProcessingContext) {
    let mut futures = FuturesUnordered::new();
    let max_name_length = context.max_name_length;

    for (repo_name, repo_path) in context.repositories.iter() {
        let semaphore = std::sync::Arc::clone(&context.semaphore);

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore).await;
            let findings = diagnose_repo(repo_path).await;
            let symbol = if findings.is_empty() { "🟢" } else { "🟡" };
            let message = if findings.is_empty() {
                "healthy".to_string()
            } else {
                findings.join("; ")
            };
            println!(
                "{symbol} {repo_name:width$}  {message}",
                width = max_name_length
            );
        };

        futures.push(future);
    }

    while futures.next().await.is_some() {}

    if let Ok(statuses) = crate::subrepo::status::analyze_subrepos() {
        if statuses.iter().any(|status| status.has_drift) {
            crate::subrepo::status::display_drift_summary(&statuses);
        }
    }

    println!();
}

async fn diagnose_repo(path: &std::path::Path) -> Vec<String> {
    let mut findings = Vec::new();

    match run_git(path, &["rev-parse", "--abbrev-ref", "HEAD"]).await {
        Ok((true, branch, _)) if branch == "HEAD" => findings.push("detached HEAD".to_string()),
        Ok((true, _, _)) => {}
        Ok((false, _, stderr)) => findings.push(format!("branch check failed: {stderr}")),
        Err(e) => findings.push(format!("branch check failed: {e}")),
    }

    match run_git(path, &["remote"]).await {
        Ok((true, remotes, _)) if remotes.trim().is_empty() => {
            findings.push("no remote".to_string());
        }
        Ok((true, _, _)) => {}
        Ok((false, _, stderr)) => findings.push(format!("remote check failed: {stderr}")),
        Err(e) => findings.push(format!("remote check failed: {e}")),
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

    findings
}
