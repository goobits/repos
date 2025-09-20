//! Repository synchronization command implementation
//!
//! This module handles the core sync functionality - discovering repositories
//! and pushing any unpushed commits to their upstream remotes.

use anyhow::Result;

use crate::core::{
    create_processing_context, init_command, set_terminal_title, set_terminal_title_and_flush,
    NO_REPOS_MESSAGE,
};
use crate::git::check_repo;

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";
const SYNCING_MESSAGE: &str = "syncing...";

/// Handles the repository sync command
pub async fn handle_sync_command(force_push: bool) -> Result<()> {
    // Set terminal title to indicate sync-repos is running
    set_terminal_title("🚀 sync-repos");

    let (start_time, repos) = init_command(SCANNING_MESSAGE);

    if repos.is_empty() {
        println!("\r{}", NO_REPOS_MESSAGE);
        // Set terminal title to green checkbox to indicate completion
        set_terminal_title_and_flush("✅ sync-repos");
        return Ok(());
    }

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    print!(
        "\r🚀 Syncing {} {}                    \n",
        total_repos, repo_word
    );
    println!();

    // Create processing context
    let context = match create_processing_context(repos, start_time) {
        Ok(context) => context,
        Err(e) => {
            // If context creation fails, set completion title and return error
            set_terminal_title_and_flush("✅ sync-repos");
            return Err(e);
        }
    };

    // Process all repositories concurrently
    process_sync_repositories(context, force_push).await;

    // Set terminal title to green checkbox to indicate completion
    set_terminal_title_and_flush("✅ sync-repos");

    Ok(())
}

/// Processes all repositories concurrently for synchronization
async fn process_sync_repositories(
    context: crate::core::ProcessingContext,
    force_push: bool,
) {
    use crate::core::{acquire_semaphore_permit, acquire_stats_lock, create_progress_bar};
    use futures::stream::{FuturesUnordered, StreamExt};
    use indicatif::{ProgressBar, ProgressStyle};

    let mut futures = FuturesUnordered::new();

    // First, create all repository progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in &context.repositories {
        let progress_bar = create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
        progress_bar.set_message(SYNCING_MESSAGE);
        repo_progress_bars.push(progress_bar);
    }

    // Add a blank line before the footer
    let separator_pb = context.multi_progress.add(ProgressBar::new(0));
    separator_pb.set_style(ProgressStyle::default_bar().template(" ").unwrap());
    separator_pb.finish();

    // Create the footer progress bar
    let footer_pb = context.multi_progress.add(ProgressBar::new(0));
    let footer_style = ProgressStyle::default_bar()
        .template("{wide_msg}")
        .expect("Failed to create footer progress style");
    footer_pb.set_style(footer_style);

    // Initial footer display
    let initial_stats = crate::core::SyncStatistics::new();
    let initial_summary = initial_stats.generate_summary(context.total_repos, context.start_time.elapsed());
    footer_pb.set_message(initial_summary);

    // Add another blank line after the footer
    let separator_pb2 = context.multi_progress.add(ProgressBar::new(0));
    separator_pb2.set_style(ProgressStyle::default_bar().template(" ").unwrap());
    separator_pb2.finish();

    // Extract values we need in the async closures before moving context.repositories
    let max_name_length = context.max_name_length;
    let start_time = context.start_time;
    let total_repos = context.total_repos;

    for ((repo_name, repo_path), progress_bar) in context.repositories.into_iter().zip(repo_progress_bars) {
        let stats_clone = std::sync::Arc::clone(&context.statistics);
        let semaphore_clone = std::sync::Arc::clone(&context.semaphore);
        let footer_clone = footer_pb.clone();

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore_clone).await;

            let (status, message, has_uncommitted_changes) =
                check_repo(&repo_path, force_push).await;

            let display_message = if has_uncommitted_changes
                && matches!(
                    status,
                    crate::git::Status::Synced | crate::git::Status::Pushed
                ) {
                format!("{} (uncommitted changes)", message)
            } else {
                message.clone()
            };

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                repo_name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<10}   {}", status.text(), display_message));
            progress_bar.finish();

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
            let summary = stats_guard.generate_summary(total_repos, duration);
            footer_clone.set_message(summary);
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
