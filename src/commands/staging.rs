//! Repository staging command implementation
//!
//! This module handles staging operations across multiple repositories:
//! - Stage files matching patterns
//! - Unstage files matching patterns
//! - Show staging status across repositories
//! - Commit staged changes across repositories

use anyhow::Result;

use crate::core::{
    create_processing_context, init_command, set_terminal_title, set_terminal_title_and_flush,
    NO_REPOS_MESSAGE, GIT_CONCURRENT_CAP,
};
use crate::git::{
    commit_changes, get_staging_status, has_staged_changes, stage_files, unstage_files, Status,
};

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";
const STAGING_MESSAGE: &str = "staging...";
const UNSTAGING_MESSAGE: &str = "unstaging...";
const STATUS_MESSAGE: &str = "checking status...";
const COMMITTING_MESSAGE: &str = "committing...";

/// Handles the repository stage command
pub async fn handle_stage_command(pattern: String) -> Result<()> {
    // Set terminal title to indicate repos is running
    set_terminal_title("🚀 repos stage");

    let (start_time, repos) = init_command(SCANNING_MESSAGE);

    if repos.is_empty() {
        println!("\r{}", NO_REPOS_MESSAGE);
        // Set terminal title to green checkbox to indicate completion
        set_terminal_title_and_flush("✅ repos stage");
        return Ok(());
    }

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    print!(
        "\r🚀 Staging {} in {} {}                    \n",
        pattern, total_repos, repo_word
    );
    println!();

    // Create processing context
    let context = match create_processing_context(repos, start_time, GIT_CONCURRENT_CAP) {
        Ok(context) => context,
        Err(e) => {
            // If context creation fails, set completion title and return error
            set_terminal_title_and_flush("✅ repos stage");
            return Err(e);
        }
    };

    // Process all repositories concurrently
    process_staging_repositories(context, pattern, true).await;

    // Set terminal title to green checkbox to indicate completion
    set_terminal_title_and_flush("✅ repos stage");

    Ok(())
}

/// Handles the repository unstage command
pub async fn handle_unstage_command(pattern: String) -> Result<()> {
    // Set terminal title to indicate repos is running
    set_terminal_title("🚀 repos unstage");

    let (start_time, repos) = init_command(SCANNING_MESSAGE);

    if repos.is_empty() {
        println!("\r{}", NO_REPOS_MESSAGE);
        // Set terminal title to green checkbox to indicate completion
        set_terminal_title_and_flush("✅ repos unstage");
        return Ok(());
    }

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    print!(
        "\r🚀 Unstaging {} in {} {}                    \n",
        pattern, total_repos, repo_word
    );
    println!();

    // Create processing context
    let context = match create_processing_context(repos, start_time, GIT_CONCURRENT_CAP) {
        Ok(context) => context,
        Err(e) => {
            // If context creation fails, set completion title and return error
            set_terminal_title_and_flush("✅ repos unstage");
            return Err(e);
        }
    };

    // Process all repositories concurrently
    process_staging_repositories(context, pattern, false).await;

    // Set terminal title to green checkbox to indicate completion
    set_terminal_title_and_flush("✅ repos unstage");

    Ok(())
}

/// Handles the repository staging status command
pub async fn handle_staging_status_command() -> Result<()> {
    // Set terminal title to indicate repos is running
    set_terminal_title("🚀 repos status");

    let (start_time, repos) = init_command(SCANNING_MESSAGE);

    if repos.is_empty() {
        println!("\r{}", NO_REPOS_MESSAGE);
        // Set terminal title to green checkbox to indicate completion
        set_terminal_title_and_flush("✅ repos status");
        return Ok(());
    }

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    print!(
        "\r🚀 Checking status of {} {}                    \n",
        total_repos, repo_word
    );
    println!();

    // Create processing context
    let context = match create_processing_context(repos, start_time, GIT_CONCURRENT_CAP) {
        Ok(context) => context,
        Err(e) => {
            // If context creation fails, set completion title and return error
            set_terminal_title_and_flush("✅ repos status");
            return Err(e);
        }
    };

    // Process all repositories concurrently for status
    process_status_repositories(context).await;

    // Set terminal title to green checkbox to indicate completion
    set_terminal_title_and_flush("✅ repos status");

    Ok(())
}

/// Processes all repositories concurrently for staging/unstaging operations
async fn process_staging_repositories(
    context: crate::core::ProcessingContext,
    pattern: String,
    is_staging: bool,
) {
    use crate::core::{acquire_semaphore_permit, acquire_stats_lock, create_progress_bar};
    use futures::stream::{FuturesUnordered, StreamExt};

    let mut futures = FuturesUnordered::new();

    // First, create all repository progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in &context.repositories {
        let progress_bar =
            create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
        let message = if is_staging {
            STAGING_MESSAGE
        } else {
            UNSTAGING_MESSAGE
        };
        progress_bar.set_message(message);
        repo_progress_bars.push(progress_bar);
    }

    // Add a blank line before the footer
    let _separator_pb = crate::core::create_separator_progress_bar(&context.multi_progress);

    // Create the footer progress bar
    let footer_pb = crate::core::create_footer_progress_bar(&context.multi_progress);

    // Initial footer display
    let initial_stats = crate::core::SyncStatistics::new();
    let initial_summary =
        initial_stats.generate_summary(context.total_repos, context.start_time.elapsed());
    footer_pb.set_message(initial_summary);

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
        let pattern_clone = pattern.clone();

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore_clone).await;

            let (status, message) = if is_staging {
                perform_staging_operation(&repo_path, &pattern_clone).await
            } else {
                perform_unstaging_operation(&repo_path, &pattern_clone).await
            };

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                repo_name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<12}   {}", status.text(), message));
            progress_bar.finish();

            // Update statistics based on operation result
            let mut stats_guard = acquire_stats_lock(&stats_clone);
            let repo_path_str = repo_path.to_string_lossy();
            stats_guard.update(
                &repo_name,
                &repo_path_str,
                &status,
                &message,
                false, // staging operations don't track uncommitted changes
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
    let detailed_summary = final_stats.generate_detailed_summary(false);
    if !detailed_summary.is_empty() {
        println!("\n{}", "━".repeat(70));
        println!("{}", detailed_summary);
        println!("{}", "━".repeat(70));
    }

    // Add final spacing
    println!();
}

/// Processes all repositories concurrently for status checking
async fn process_status_repositories(context: crate::core::ProcessingContext) {
    use crate::core::{acquire_semaphore_permit, create_progress_bar};
    use futures::stream::{FuturesUnordered, StreamExt};

    let mut futures = FuturesUnordered::new();

    // First, create all repository progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in &context.repositories {
        let progress_bar =
            create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
        progress_bar.set_message(STATUS_MESSAGE);
        repo_progress_bars.push(progress_bar);
    }

    // Add a blank line before results
    let _separator_pb = crate::core::create_separator_progress_bar(&context.multi_progress);

    // Extract values we need in the async closures before moving context.repositories
    let max_name_length = context.max_name_length;

    for ((repo_name, repo_path), progress_bar) in
        context.repositories.into_iter().zip(repo_progress_bars)
    {
        let semaphore_clone = std::sync::Arc::clone(&context.semaphore);

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore_clone).await;

            let status_result = get_staging_status(&repo_path).await;
            let (status, message) = match status_result {
                Ok((stdout, _)) => {
                    if stdout.trim().is_empty() {
                        (Status::NoChanges, "no changes".to_string())
                    } else {
                        let lines: Vec<&str> = stdout.trim().lines().collect();
                        let staged_count = lines
                            .iter()
                            .filter(|line| {
                                let chars: Vec<char> = line.chars().collect();
                                chars.len() >= 2 && chars[0] != ' ' && chars[0] != '?'
                            })
                            .count();
                        let unstaged_count = lines
                            .iter()
                            .filter(|line| {
                                let chars: Vec<char> = line.chars().collect();
                                chars.len() >= 2 && chars[1] != ' '
                            })
                            .count();
                        let untracked_count =
                            lines.iter().filter(|line| line.starts_with("??")).count();

                        let mut parts = Vec::new();
                        if staged_count > 0 {
                            parts.push(format!("{} staged", staged_count));
                        }
                        if unstaged_count > 0 {
                            parts.push(format!("{} unstaged", unstaged_count));
                        }
                        if untracked_count > 0 {
                            parts.push(format!("{} untracked", untracked_count));
                        }

                        if parts.is_empty() {
                            (Status::NoChanges, "no changes".to_string())
                        } else {
                            (Status::Synced, parts.join(", "))
                        }
                    }
                }
                Err(e) => (Status::StagingError, format!("error: {}", e)),
            };

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                repo_name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<12}   {}", status.text(), message));
            progress_bar.finish();
        };

        futures.push(future);
    }

    // Wait for all repository operations to complete
    while futures.next().await.is_some() {}

    // Add final spacing
    println!();
}

/// Handles the repository commit command
pub async fn handle_commit_command(message: String, include_empty: bool) -> Result<()> {
    // Set terminal title to indicate repos is running
    set_terminal_title("🚀 repos commit");

    let (start_time, repos) = init_command(SCANNING_MESSAGE);

    if repos.is_empty() {
        println!("\r{}", NO_REPOS_MESSAGE);
        // Set terminal title to green checkbox to indicate completion
        set_terminal_title_and_flush("✅ repos commit");
        return Ok(());
    }

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    print!(
        "\r🚀 Committing changes in {} {}                    \n",
        total_repos, repo_word
    );
    println!();

    // Create processing context
    let context = match create_processing_context(repos, start_time, GIT_CONCURRENT_CAP) {
        Ok(context) => context,
        Err(e) => {
            // If context creation fails, set completion title and return error
            set_terminal_title_and_flush("✅ repos commit");
            return Err(e);
        }
    };

    // Process all repositories concurrently
    process_commit_repositories(context, message, include_empty).await;

    // Set terminal title to green checkbox to indicate completion
    set_terminal_title_and_flush("✅ repos commit");

    Ok(())
}

/// Processes all repositories concurrently for commit operations
async fn process_commit_repositories(
    context: crate::core::ProcessingContext,
    message: String,
    include_empty: bool,
) {
    use crate::core::{acquire_semaphore_permit, acquire_stats_lock, create_progress_bar};
    use futures::stream::{FuturesUnordered, StreamExt};

    let mut futures = FuturesUnordered::new();

    // First, create all repository progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in &context.repositories {
        let progress_bar =
            create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
        progress_bar.set_message(COMMITTING_MESSAGE);
        repo_progress_bars.push(progress_bar);
    }

    // Add a blank line before the footer
    let _separator_pb = crate::core::create_separator_progress_bar(&context.multi_progress);

    // Create the footer progress bar
    let footer_pb = crate::core::create_footer_progress_bar(&context.multi_progress);

    // Initial footer display
    let initial_stats = crate::core::SyncStatistics::new();
    let initial_summary =
        initial_stats.generate_summary(context.total_repos, context.start_time.elapsed());
    footer_pb.set_message(initial_summary);

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
        let message_clone = message.clone();

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore_clone).await;

            let (status, message) =
                perform_commit_operation(&repo_path, &message_clone, include_empty).await;

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                repo_name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<12}   {}", status.text(), message));
            progress_bar.finish();

            // Update statistics based on operation result
            let mut stats_guard = acquire_stats_lock(&stats_clone);
            let repo_path_str = repo_path.to_string_lossy();
            stats_guard.update(
                &repo_name,
                &repo_path_str,
                &status,
                &message,
                false, // commit operations don't track uncommitted changes
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
    let detailed_summary = final_stats.generate_detailed_summary(false);
    if !detailed_summary.is_empty() {
        println!("\n{}", "━".repeat(70));
        println!("{}", detailed_summary);
        println!("{}", "━".repeat(70));
    }

    // Add final spacing
    println!();
}

/// Performs a staging operation on a single repository
async fn perform_staging_operation(repo_path: &std::path::Path, pattern: &str) -> (Status, String) {
    use crate::core::clean_error_message;

    match stage_files(repo_path, pattern).await {
        Ok((true, _, _)) => (Status::Staged, format!("staged {}", pattern)),
        Ok((false, _, stderr)) => {
            let error_message = clean_error_message(&stderr);
            if error_message.contains("pathspec") && error_message.contains("did not match") {
                (Status::NoChanges, format!("no files match {}", pattern))
            } else {
                (Status::StagingError, error_message)
            }
        }
        Err(e) => {
            let error_message = clean_error_message(&e.to_string());
            (Status::StagingError, error_message)
        }
    }
}

/// Performs a commit operation on a single repository
async fn perform_commit_operation(
    repo_path: &std::path::Path,
    message: &str,
    include_empty: bool,
) -> (Status, String) {
    use crate::core::clean_error_message;

    // First check if there are staged changes (unless we're allowing empty commits)
    if !include_empty {
        match has_staged_changes(repo_path).await {
            Ok(false) => {
                return (Status::NoChanges, "no staged changes".to_string());
            }
            Ok(true) => {
                // Has staged changes, proceed with commit
            }
            Err(e) => {
                let error_message = clean_error_message(&e.to_string());
                return (
                    Status::CommitError,
                    format!("error checking changes: {}", error_message),
                );
            }
        }
    }

    // Perform the commit
    match commit_changes(repo_path, message, include_empty).await {
        Ok((true, stdout, _)) => {
            // Parse commit output to get commit hash (first 7 chars of first line usually)
            let commit_info = if let Some(first_line) = stdout.lines().next() {
                if first_line.len() > 7 {
                    &first_line[0..7]
                } else {
                    "committed"
                }
            } else {
                "committed"
            };
            (Status::Committed, format!("committed {}", commit_info))
        }
        Ok((false, _, stderr)) => {
            let error_message = clean_error_message(&stderr);
            if error_message.contains("nothing to commit")
                || error_message.contains("no changes added")
            {
                (Status::NoChanges, "nothing to commit".to_string())
            } else {
                (Status::CommitError, error_message)
            }
        }
        Err(e) => {
            let error_message = clean_error_message(&e.to_string());
            (Status::CommitError, error_message)
        }
    }
}

/// Performs an unstaging operation on a single repository
async fn perform_unstaging_operation(
    repo_path: &std::path::Path,
    pattern: &str,
) -> (Status, String) {
    use crate::core::clean_error_message;

    match unstage_files(repo_path, pattern).await {
        Ok((true, _, _)) => (Status::Unstaged, format!("unstaged {}", pattern)),
        Ok((false, _, stderr)) => {
            let error_message = clean_error_message(&stderr);
            if error_message.contains("pathspec") && error_message.contains("did not match") {
                (
                    Status::NoChanges,
                    format!("no staged files match {}", pattern),
                )
            } else {
                (Status::StagingError, error_message)
            }
        }
        Err(e) => {
            let error_message = clean_error_message(&e.to_string());
            (Status::StagingError, error_message)
        }
    }
}
