//! Intent-driven save command.
//!
//! `repos save` is the safe daily workflow: stage tracked changes, commit, and
//! push. Untracked files are opt-in to avoid committing local scratch files or
//! secrets across a repository fleet.

use anyhow::Result;
use futures::stream::{FuturesUnordered, StreamExt};

use crate::core::{
    acquire_semaphore_permit, clean_error_message, create_processing_context, init_command,
    set_terminal_title, set_terminal_title_and_flush, GIT_CONCURRENT_CAP, NO_REPOS_MESSAGE,
};
use crate::git::{
    commit_changes, fetch_and_analyze, get_staging_status, has_staged_changes, push_if_needed,
    stage_all_changes, stage_tracked_changes, Status,
};

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";

/// Handles `repos save`.
pub async fn handle_save_command(
    message: String,
    include_untracked: bool,
    all: bool,
    auto_upstream: bool,
    dry_run: bool,
) -> Result<()> {
    set_terminal_title("💾 repos save");

    let (start_time, repos) = init_command(SCANNING_MESSAGE).await;

    if repos.is_empty() {
        println!("\r{NO_REPOS_MESSAGE}");
        set_terminal_title_and_flush("✅ repos save");
        return Ok(());
    }

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    let action = if dry_run { "Planning save" } else { "Saving" };
    print!("\r💾 {action} across {total_repos} {repo_word}                    \n\n");

    let context =
        match create_processing_context(std::sync::Arc::new(repos), start_time, GIT_CONCURRENT_CAP)
        {
            Ok(context) => context,
            Err(e) => {
                set_terminal_title_and_flush("✅ repos save");
                return Err(e);
            }
        };

    process_save_repositories(context, message, include_untracked || all, auto_upstream, dry_run)
        .await;

    set_terminal_title_and_flush("✅ repos save");
    Ok(())
}

async fn process_save_repositories(
    context: crate::core::ProcessingContext,
    commit_message: String,
    include_untracked: bool,
    auto_upstream: bool,
    dry_run: bool,
) {
    use crate::core::{acquire_stats_lock, create_progress_bar};

    let mut progress_bars = Vec::new();
    for (repo_name, _) in context.repositories.iter() {
        let progress_bar =
            create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
        progress_bar.set_message(if dry_run { "planning..." } else { "saving..." });
        progress_bars.push(progress_bar);
    }

    let _separator_pb = crate::core::create_separator_progress_bar(&context.multi_progress);
    let footer_pb = crate::core::create_footer_progress_bar(&context.multi_progress);
    footer_pb.set_message("💾 0 Saved  🟢 0 Synced  🔴 0 Failed  🟠 0 Skipped".to_string());
    let _separator_pb2 = crate::core::create_separator_progress_bar(&context.multi_progress);

    let max_name_length = context.max_name_length;
    let start_time = context.start_time;
    let total_repos = context.total_repos;

    let mut futures = FuturesUnordered::new();
    for ((repo_name, repo_path), progress_bar) in context.repositories.iter().zip(progress_bars) {
        let semaphore = std::sync::Arc::clone(&context.semaphore);
        let stats = std::sync::Arc::clone(&context.statistics);
        let footer = footer_pb.clone();
        let commit_message = commit_message.clone();

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore).await;

            let (status, message, has_uncommitted) = save_one_repo(
                repo_path,
                &commit_message,
                include_untracked,
                auto_upstream,
                dry_run,
            )
            .await;

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                repo_name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<12}   {}", status.text(), message));
            progress_bar.finish();

            let stats_guard = acquire_stats_lock(&stats);
            stats_guard.update(
                repo_name,
                &repo_path.to_string_lossy(),
                &status,
                &message,
                has_uncommitted,
            );
            footer.set_message(stats_guard.generate_summary(total_repos, start_time.elapsed()));
        };

        futures.push(future);
    }

    while futures.next().await.is_some() {}

    footer_pb.finish();

    let final_stats = acquire_stats_lock(&context.statistics);
    let detailed_summary = final_stats.generate_detailed_summary(false);
    if !detailed_summary.is_empty() {
        println!("\n{}", "━".repeat(70));
        println!("{detailed_summary}");
        println!("{}", "━".repeat(70));
    }
    println!();
}

async fn save_one_repo(
    repo_path: &std::path::Path,
    commit_message: &str,
    include_untracked: bool,
    auto_upstream: bool,
    dry_run: bool,
) -> (Status, String, bool) {
    let status = match get_staging_status(repo_path).await {
        Ok((stdout, _)) => stdout,
        Err(e) => return (Status::StagingError, format!("status failed: {e}"), false),
    };

    let has_tracked_changes = status.lines().any(has_tracked_change);
    let has_untracked_changes = status.lines().any(|line| line.starts_with("??"));

    if !has_tracked_changes && has_untracked_changes && !include_untracked {
        return (
            Status::NoChanges,
            "only untracked changes; pass --include-untracked".to_string(),
            true,
        );
    }

    if !has_tracked_changes && !has_untracked_changes {
        return (Status::Synced, "clean".to_string(), false);
    }

    if dry_run {
        let stage_mode = if include_untracked {
            "stage all changes"
        } else {
            "stage tracked changes"
        };
        return (
            Status::Staged,
            format!("{stage_mode}, commit, push"),
            has_tracked_changes || has_untracked_changes,
        );
    }

    let stage_result = if include_untracked {
        stage_all_changes(repo_path).await
    } else {
        stage_tracked_changes(repo_path).await
    };

    match stage_result {
        Ok((true, _, _)) => {}
        Ok((false, _, stderr)) => {
            return (Status::StagingError, clean_error_message(&stderr), true);
        }
        Err(e) => return (Status::StagingError, format!("stage failed: {e}"), true),
    }

    match has_staged_changes(repo_path).await {
        Ok(true) => {}
        Ok(false) => return (Status::NoChanges, "nothing staged".to_string(), true),
        Err(e) => return (Status::StagingError, format!("stage check failed: {e}"), true),
    }

    match commit_changes(repo_path, commit_message, false).await {
        Ok((true, _, _)) => {}
        Ok((false, _, stderr)) => {
            return (Status::CommitError, clean_error_message(&stderr), true);
        }
        Err(e) => return (Status::CommitError, format!("commit failed: {e}"), true),
    }

    let fetch_result = fetch_and_analyze(repo_path, auto_upstream).await;
    let (push_status, push_message, has_uncommitted) =
        push_if_needed(repo_path, &fetch_result, auto_upstream).await;

    match push_status {
        Status::Pushed | Status::Synced => (
            Status::Committed,
            format!("committed; {push_message}"),
            has_uncommitted,
        ),
        other => (
            other,
            format!("committed; push skipped: {push_message}"),
            has_uncommitted,
        ),
    }
}

fn has_tracked_change(line: &str) -> bool {
    if line.starts_with("??") || line.len() < 2 {
        return false;
    }

    let bytes = line.as_bytes();
    bytes[0] != b' ' || bytes[1] != b' '
}
