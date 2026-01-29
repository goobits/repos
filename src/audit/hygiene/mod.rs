//! Repository hygiene checking for detecting improperly committed files

pub mod rules;
pub mod scanner;
pub mod report;

use std::sync::Arc;
use futures::stream::{FuturesUnordered, StreamExt};
use crate::core::{create_progress_bar, GenericProcessingContext};

// Re-export key types and functions
pub use report::{HygieneStatistics, HygieneViolation, ViolationType};
pub use scanner::check_repo_hygiene;

/// Helper function to safely acquire a semaphore permit
async fn acquire_semaphore_permit(
    semaphore: &tokio::sync::Semaphore,
) -> tokio::sync::SemaphorePermit<'_> {
    semaphore
        .acquire()
        .await
        .expect("Failed to acquire semaphore permit for concurrent hygiene operations")
}

/// Processes all repositories concurrently for hygiene checking
pub async fn process_hygiene_repositories(context: GenericProcessingContext<HygieneStatistics>) {
    let mut futures = FuturesUnordered::new();

    // Create all repository progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in context.repositories.iter() {
        let progress_bar =
            create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
        progress_bar.set_message("checking hygiene...");
        repo_progress_bars.push(progress_bar);
    }

    // Add a blank line before the footer
    let _separator_pb = crate::core::create_separator_progress_bar(&context.multi_progress);

    // Create the footer progress bar
    let footer_pb = crate::core::create_footer_progress_bar(&context.multi_progress);

    // Initial footer display
    let initial_stats = HygieneStatistics::new();
    let initial_summary =
        initial_stats.generate_summary(context.total_repos, context.start_time.elapsed());
    footer_pb.set_message(initial_summary);

    // Add another blank line after the footer
    let _separator_pb2 = crate::core::create_separator_progress_bar(&context.multi_progress);

    // Extract values we need in the async closures
    let max_name_length = context.max_name_length;
    let start_time = context.start_time;
    let total_repos = context.total_repos;

    for ((repo_name, repo_path), progress_bar) in
        context.repositories.iter().zip(repo_progress_bars)
    {
        let stats_clone = Arc::clone(&context.statistics);
        let semaphore_clone = Arc::clone(&context.semaphore);
        let footer_clone = footer_pb.clone();

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore_clone).await;

            let (status, message, violations) = check_repo_hygiene(&repo_path).await;

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                repo_name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<10}   {}", status.text(), message));
            progress_bar.finish();

            // Update statistics
            let mut stats_guard = stats_clone.lock().expect("Failed to acquire stats lock");
            let repo_path_str = repo_path.to_string_lossy();
            stats_guard.update(&repo_name, &repo_path_str, &status, &message, violations);

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
    let final_stats = context
        .statistics
        .lock()
        .expect("Failed to acquire stats lock");
    let detailed_summary = final_stats.generate_detailed_summary();
    if !detailed_summary.is_empty() {
        println!("\n{}", "━".repeat(70));
        println!("{detailed_summary}");
        println!("{}", "━".repeat(70));
    }

    // Add final spacing
    println!();
}
