//! Repository push command implementation
//!
//! This module handles the core push functionality - discovering repositories
//! and pushing any unpushed commits to their upstream remotes.

use anyhow::Result;

use crate::core::{
    create_processing_context, init_command, set_terminal_title, set_terminal_title_and_flush,
    NO_REPOS_MESSAGE,
};
use crate::git::check_repo;

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";
const PUSHING_MESSAGE: &str = "pushing...";

/// Handles the repository push command
pub async fn handle_push_command(force_push: bool, verbose: bool) -> Result<()> {
    // Set terminal title to indicate repos is running
    set_terminal_title("🚀 repos");

    let (start_time, repos) = init_command(SCANNING_MESSAGE);

    if repos.is_empty() {
        println!("\r{}", NO_REPOS_MESSAGE);
        // Set terminal title to green checkbox to indicate completion
        set_terminal_title_and_flush("✅ repos");
        return Ok(());
    }

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    print!(
        "\r🚀 Pushing {} {}                    \n",
        total_repos, repo_word
    );
    println!();

    // Create processing context
    let context = match create_processing_context(repos, start_time) {
        Ok(context) => context,
        Err(e) => {
            // If context creation fails, set completion title and return error
            set_terminal_title_and_flush("✅ repos");
            return Err(e);
        }
    };

    // Process all repositories concurrently
    process_push_repositories(context, force_push, verbose).await;

    // Set terminal title to green checkbox to indicate completion
    set_terminal_title_and_flush("✅ repos");

    Ok(())
}

/// Processes all repositories concurrently for pushing
async fn process_push_repositories(context: crate::core::ProcessingContext, force_push: bool, verbose: bool) {
    use crate::core::{acquire_semaphore_permit, acquire_stats_lock, create_progress_bar};
    use futures::stream::{FuturesUnordered, StreamExt};

    let mut futures = FuturesUnordered::new();

    // Create progress bars based on verbose mode
    let repo_progress_bars: Vec<_> = if verbose {
        // Verbose mode: create individual progress bars for each repo (current behavior)
        context.repositories.iter()
            .map(|(repo_name, _)| {
                let pb = create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
                pb.set_message(PUSHING_MESSAGE);
                pb
            })
            .collect()
    } else {
        // Default mode: create single updating progress bar
        use indicatif::{ProgressBar, ProgressStyle};

        let single_pb = context.multi_progress.add(
            ProgressBar::new(context.total_repos as u64)
        );
        single_pb.set_style(
            ProgressStyle::default_bar()
                .template("[{pos}/{len}] {msg}")
                .unwrap()
        );
        single_pb.set_message("🚀 Starting...");

        // Create vec of shared references to the same progress bar
        vec![single_pb; context.repositories.len()]
    };

    // Add a blank line before the footer
    let _separator_pb = crate::core::create_separator_progress_bar(&context.multi_progress);

    // Create the footer progress bar
    let footer_pb = crate::core::create_footer_progress_bar(&context.multi_progress);

    // Initial footer display
    let initial_stats = crate::core::SyncStatistics::new();
    if verbose {
        let initial_summary =
            initial_stats.generate_summary(context.total_repos, context.start_time.elapsed());
        footer_pb.set_message(initial_summary);
    } else {
        footer_pb.set_message(
            "✅ 0 Pushed  🟢 0 Synced  🔴 0 Failed  🟡 0 No Upstream  🟠 0 Skipped".to_string()
        );
    }

    // Add another blank line after the footer
    let _separator_pb2 = crate::core::create_separator_progress_bar(&context.multi_progress);

    // Extract values we need in the async closures before moving context.repositories
    let max_name_length = context.max_name_length;
    let start_time = context.start_time;
    let total_repos = context.total_repos;

    for ((repo_name, repo_path), progress_bar) in
        context.repositories.into_iter().zip(repo_progress_bars)
    {
        let stats_clone = std::sync::Arc::clone(&context.statistics);
        let semaphore_clone = std::sync::Arc::clone(&context.semaphore);
        let footer_clone = footer_pb.clone();
        let verbose_clone = verbose;

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore_clone).await;

            let (status, message, has_uncommitted_changes) =
                check_repo(&repo_path, force_push).await;

            let display_message = if has_uncommitted_changes
                && matches!(status, crate::git::Status::Synced)
            {
                format!("{} (uncommitted changes)", message)
            } else {
                message.clone()
            };

            if verbose_clone {
                // Verbose mode: update individual progress bars
                progress_bar.set_prefix(format!(
                    "{} {:width$}",
                    status.symbol(),
                    repo_name,
                    width = max_name_length
                ));
                progress_bar.set_message(format!("{:<10}   {}", status.text(), display_message));
                progress_bar.finish();
            } else {
                // Non-verbose mode: update single progress bar
                progress_bar.set_message(format!("{} {} ({})", status.symbol(), repo_name, status.text()));
                progress_bar.inc(1);
            }

            // Update statistics based on operation result
            let mut stats_guard = acquire_stats_lock(&stats_clone);
            let repo_path_str = repo_path.to_string_lossy();
            stats_guard.update(
                &repo_name,
                &repo_path_str,
                &status,
                &message,
                has_uncommitted_changes,
            );

            // Update the footer summary after each repository completes
            let duration = start_time.elapsed();
            if verbose_clone {
                let summary = stats_guard.generate_summary(total_repos, duration);
                footer_clone.set_message(summary);
            } else {
                // Non-verbose mode: show live counters
                let live_counters = format!(
                    "✅ {} Pushed  🟢 {} Synced  🔴 {} Failed  🟡 {} No Upstream  🟠 {} Skipped",
                    stats_guard.total_commits_pushed,
                    stats_guard.synced_repos,
                    stats_guard.error_repos,
                    stats_guard.no_upstream_repos.len(),
                    stats_guard.skipped_repos
                );
                footer_clone.set_message(live_counters);
            }
        };

        futures.push(future);
    }

    // Wait for all repository operations to complete
    while futures.next().await.is_some() {}

    // Finish the footer progress bar
    footer_pb.finish();

    // Print the final detailed summary if there are any issues to report
    let final_stats = acquire_stats_lock(&context.statistics);
    let detailed_summary = final_stats.generate_detailed_summary();
    if !detailed_summary.is_empty() {
        println!("\n{}", "━".repeat(70));
        println!("{}", detailed_summary);
        println!("{}", "━".repeat(70));
    }

    // Add final spacing
    println!();
}
