//! Repository staging command implementation
//!
//! This module handles staging operations across multiple repositories:
//! - Stage files matching patterns
//! - Unstage files matching patterns
//! - Show staging status across repositories
//! - Commit staged changes across repositories

mod status;

use status::process_status_repositories;
pub use status::StatusFilters;

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::core::{
    clean_error_message, create_processing_context, format_relative_repo_path, init_command,
    print_final_report, set_terminal_title, set_terminal_title_and_flush, truncate_text,
    BatchOperation, RepositoryOrder, RepositoryTopology, GIT_CONCURRENT_CAP, NO_REPOS_MESSAGE,
};
use crate::git::failure::GitFailure;
use crate::git::{
    commit_changes, get_staging_status, has_staged_changes, is_detached_head, stage_files,
    unstage_files, Status,
};
use crate::utils::compare_repository_locations;

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";
const STAGING_MESSAGE: &str = "staging...";
const UNSTAGING_MESSAGE: &str = "unstaging...";
const STATUS_MESSAGE: &str = "checking status...";
const COMMITTING_MESSAGE: &str = "committing...";
const RESET: &str = "\x1b[0m";
const BOLD_BLUE: &str = "\x1b[1;38;5;75m";
const BOLD_PURPLE: &str = "\x1b[1;38;5;141m";
const GREEN: &str = "\x1b[1;38;5;114m";
const YELLOW: &str = "\x1b[1;38;5;221m";
const RED: &str = "\x1b[1;38;5;203m";
const DIM: &str = "\x1b[2m";

/// Handles the repository stage command
pub async fn handle_stage_command(pattern: String) -> Result<()> {
    let Some(context) = prepare_batch_command(
        "🚀 repos stage",
        "✅ repos stage",
        format!("Staging {pattern}"),
    )
    .await?
    else {
        return Ok(());
    };

    process_staging_repositories(context, pattern, true).await?;
    set_terminal_title_and_flush("✅ repos stage");
    Ok(())
}

/// Handles the repository unstage command
pub async fn handle_unstage_command(pattern: String) -> Result<()> {
    let Some(context) = prepare_batch_command(
        "🚀 repos unstage",
        "✅ repos unstage",
        format!("Unstaging {pattern}"),
    )
    .await?
    else {
        return Ok(());
    };

    process_staging_repositories(context, pattern, false).await?;
    set_terminal_title_and_flush("✅ repos unstage");
    Ok(())
}

async fn prepare_batch_command(
    running_title: &str,
    done_title: &str,
    action: String,
) -> Result<Option<crate::core::ProcessingContext>> {
    set_terminal_title(running_title);

    let (start_time, repos) = init_command(SCANNING_MESSAGE).await;
    if repos.is_empty() {
        println!("\r{NO_REPOS_MESSAGE}");
        set_terminal_title_and_flush(done_title);
        return Ok(None);
    }

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    print!("\r🚀 {action} in {total_repos} {repo_word}                    \n");
    println!();

    match create_processing_context(std::sync::Arc::new(repos), start_time, GIT_CONCURRENT_CAP) {
        Ok(context) => Ok(Some(context)),
        Err(e) => {
            set_terminal_title_and_flush(done_title);
            Err(e)
        }
    }
}

/// Handles the repository staging status command
pub async fn handle_staging_status_command(
    targets: Vec<String>,
    filters: StatusFilters,
) -> Result<()> {
    // Set terminal title to indicate repos is running
    set_terminal_title("🚀 repos status");

    let (start_time, mut repos) = init_command(SCANNING_MESSAGE).await;
    repos = filter_status_repositories(repos, &targets);

    if repos.is_empty() {
        if targets.is_empty() {
            println!("\r{NO_REPOS_MESSAGE}");
        } else {
            println!("\rNo repositories matched: {}", targets.join(", "));
        }
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
    print!("\r🚀 Checking status of {total_repos} {repo_word}                    \n");
    println!();

    // Create processing context
    let context =
        match create_processing_context(std::sync::Arc::new(repos), start_time, GIT_CONCURRENT_CAP)
        {
            Ok(context) => context,
            Err(e) => {
                // If context creation fails, set completion title and return error
                set_terminal_title_and_flush("✅ repos status");
                return Err(e);
            }
        };

    // Process all repositories concurrently for status
    let failed = process_status_repositories(context, filters).await;

    // Set terminal title to green checkbox to indicate completion
    set_terminal_title_and_flush("✅ repos status");

    if failed > 0 {
        anyhow::bail!("{failed} repositories failed status inspection");
    }

    Ok(())
}

fn filter_status_repositories(
    repos: Vec<(String, PathBuf)>,
    targets: &[String],
) -> Vec<(String, PathBuf)> {
    if targets.is_empty() {
        return repos;
    }

    let normalized_targets = targets
        .iter()
        .map(|target| normalize_target(target))
        .collect::<Vec<_>>();

    repos
        .into_iter()
        .filter(|(repo_name, repo_path)| {
            normalized_targets.iter().any(|target| {
                repo_name == target
                    || repo_path_matches_target(repo_path, target)
                    || repo_path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name == target)
            })
        })
        .collect()
}

fn normalize_target(target: &str) -> String {
    target
        .trim_end_matches('/')
        .trim_start_matches("./")
        .to_string()
}

fn repo_path_matches_target(repo_path: &Path, target: &str) -> bool {
    let normalized_path = repo_path
        .to_string_lossy()
        .trim_start_matches("./")
        .trim_end_matches('/')
        .to_string();

    normalized_path == target || normalized_path.ends_with(&format!("/{target}"))
}

/// Processes all repositories concurrently for staging/unstaging operations
async fn process_staging_repositories(
    context: crate::core::ProcessingContext,
    pattern: String,
    is_staging: bool,
) -> Result<()> {
    use crate::core::{acquire_semaphore_permit, create_progress_bar};
    use futures::stream::{FuturesUnordered, StreamExt};

    let mut futures = FuturesUnordered::new();
    let operation = if is_staging {
        BatchOperation::Stage
    } else {
        BatchOperation::Unstage
    };

    // First, create all repository progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in context.repositories.iter() {
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
    let initial_summary = initial_stats.generate_batch_live_summary(operation, context.total_repos);
    footer_pb.set_message(initial_summary);

    // Add another blank line after the footer
    let _separator_pb2 = crate::core::create_separator_progress_bar(&context.multi_progress);

    // Extract values we need in the async closures before moving context.repositories
    let max_name_length = context.max_name_length;
    let start_time = context.start_time;
    let total_repos = context.total_repos;

    for ((repo_name, repo_path), progress_bar) in
        context.repositories.iter().zip(repo_progress_bars)
    {
        let stats_clone = std::sync::Arc::clone(&context.statistics);
        let semaphore_clone = std::sync::Arc::clone(&context.semaphore);
        let footer_clone = footer_pb.clone();
        let pattern_clone = pattern.clone();

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore_clone).await;

            let (status, message) = if is_staging {
                perform_staging_operation(repo_path, &pattern_clone).await
            } else {
                perform_unstaging_operation(repo_path, &pattern_clone).await
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
            let repo_path_str = repo_path.to_string_lossy();
            stats_clone.update(
                repo_name,
                &repo_path_str,
                &status,
                &message,
                false, // staging operations don't track uncommitted changes
            );

            // Update the footer summary after each repository completes
            let summary = stats_clone.generate_batch_live_summary(operation, total_repos);
            footer_clone.set_message(summary);
        };

        futures.push(future);
    }

    // Wait for all repository operations to complete
    while futures.next().await.is_some() {}

    // Finish the footer progress bar
    footer_pb.finish();

    let final_stats = context.statistics.as_ref();
    println!(
        "\n{}\n",
        final_stats.generate_batch_report(operation, start_time.elapsed())
    );

    let error_count = final_stats
        .error_repos
        .load(std::sync::atomic::Ordering::Relaxed);
    if error_count > 0 {
        anyhow::bail!("{error_count} repositories failed staging operations");
    }

    Ok(())
}

/// Handles the repository commit command
pub async fn handle_commit_command(message: String, include_empty: bool) -> Result<()> {
    let Some(context) = prepare_batch_command(
        "🚀 repos commit",
        "✅ repos commit",
        "Committing changes".to_string(),
    )
    .await?
    else {
        return Ok(());
    };

    process_commit_repositories(context, message, include_empty).await?;
    set_terminal_title_and_flush("✅ repos commit");
    Ok(())
}

/// Commits child-first dependency waves, concurrently within each wave.
async fn process_commit_repositories(
    context: crate::core::ProcessingContext,
    message: String,
    include_empty: bool,
) -> Result<()> {
    use crate::core::{acquire_semaphore_permit, create_progress_bar};
    use futures::stream::{FuturesUnordered, StreamExt};

    let operation = BatchOperation::Commit;

    // First, create all repository progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in context.repositories.iter() {
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
    let initial_summary = initial_stats.generate_batch_live_summary(operation, context.total_repos);
    footer_pb.set_message(initial_summary);

    // Add another blank line after the footer
    let _separator_pb2 = crate::core::create_separator_progress_bar(&context.multi_progress);

    // Extract values we need in the async closures before moving context.repositories
    let max_name_length = context.max_name_length;
    let start_time = context.start_time;
    let total_repos = context.total_repos;

    let topology = RepositoryTopology::new(&context.repositories);
    let mut completed = vec![None; context.total_repos];
    for wave in topology.waves(RepositoryOrder::ChildrenFirst) {
        let mut futures = FuturesUnordered::new();
        for index in wave {
            let (repo_name, repo_path) = &context.repositories[index];
            let progress_bar = repo_progress_bars[index].clone();
            let committed_gitlinks = topology
                .gitlink_children(index)
                .iter()
                .filter(|child| completed[**child] == Some(Status::Committed))
                .map(|child| context.repositories[*child].1.clone())
                .collect::<Vec<_>>();
            let gitlink_inspection_error =
                topology.gitlink_inspection_error(index).map(str::to_string);
            let stats_clone = std::sync::Arc::clone(&context.statistics);
            let semaphore_clone = std::sync::Arc::clone(&context.semaphore);
            let footer_clone = footer_pb.clone();
            let message_clone = message.clone();

            let future = async move {
                let _permit = acquire_semaphore_permit(&semaphore_clone).await;

                let (status, message) = perform_commit_operation(
                    repo_path,
                    &message_clone,
                    include_empty,
                    &committed_gitlinks,
                    gitlink_inspection_error.as_deref(),
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

                // Update statistics based on operation result
                let repo_path_str = repo_path.to_string_lossy();
                stats_clone.update(
                    repo_name,
                    &repo_path_str,
                    &status,
                    &message,
                    false, // commit operations don't track uncommitted changes
                );

                // Update the footer summary after each repository completes
                let summary = stats_clone.generate_batch_live_summary(operation, total_repos);
                footer_clone.set_message(summary);
                (index, status)
            };

            futures.push(future);
        }

        while let Some((index, status)) = futures.next().await {
            completed[index] = Some(status);
        }
    }

    // Finish the footer progress bar
    footer_pb.finish();

    let final_stats = context.statistics.as_ref();
    println!(
        "\n{}\n",
        final_stats.generate_batch_report(operation, start_time.elapsed())
    );

    let error_count = final_stats
        .error_repos
        .load(std::sync::atomic::Ordering::Relaxed);
    if error_count > 0 {
        anyhow::bail!("{error_count} repositories failed to commit");
    }

    Ok(())
}

/// Performs a staging operation on a single repository
async fn perform_staging_operation(repo_path: &std::path::Path, pattern: &str) -> (Status, String) {
    use crate::core::clean_error_message;

    match stage_files(repo_path, pattern).await {
        Ok((true, _, _)) => (Status::Staged, format!("staged {pattern}")),
        Ok((false, _, stderr)) => {
            if stderr.contains("pathspec") && stderr.contains("did not match") {
                (Status::NoChanges, format!("no files match {pattern}"))
            } else {
                (Status::StagingError, clean_error_message(&stderr))
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
    committed_gitlinks: &[PathBuf],
    gitlink_inspection_error: Option<&str>,
) -> (Status, String) {
    use crate::core::clean_error_message;

    if let Some(error) = gitlink_inspection_error {
        return (
            Status::CommitError,
            format!("submodule relationship inspection failed: {error}"),
        );
    }

    match is_detached_head(repo_path).await {
        Ok(true) => {
            return (
                Status::Skip,
                "detached HEAD; checkout a branch before commit".to_string(),
            );
        }
        Ok(false) => {}
        Err(e) => {
            return (
                Status::CommitError,
                format!(
                    "branch check failed: {}",
                    clean_error_message(&e.to_string())
                ),
            );
        }
    }

    for child_path in committed_gitlinks {
        let Ok(relative) = child_path.strip_prefix(repo_path) else {
            return (
                Status::CommitError,
                format!(
                    "failed to refresh submodule pointer outside parent: {}",
                    child_path.display()
                ),
            );
        };
        let Some(relative) = relative.to_str() else {
            return (
                Status::CommitError,
                "failed to refresh non-UTF-8 submodule path".to_string(),
            );
        };
        match crate::git::operations::run_git(repo_path, &["add", "--", relative]).await {
            Ok((true, _, _)) => {}
            Ok((false, _, stderr)) => {
                return (
                    Status::CommitError,
                    format!(
                        "failed to refresh submodule pointer: {}",
                        clean_error_message(&stderr)
                    ),
                );
            }
            Err(error) => {
                return (
                    Status::CommitError,
                    format!("failed to refresh submodule pointer: {error}"),
                );
            }
        }
    }

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
                    format!("error checking changes: {error_message}"),
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
            (Status::Committed, format!("committed {commit_info}"))
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
        Ok((true, _, _)) => (Status::Unstaged, format!("unstaged {pattern}")),
        Ok((false, _, stderr)) => {
            if stderr.contains("pathspec") && stderr.contains("did not match") {
                (
                    Status::NoChanges,
                    format!("no staged files match {pattern}"),
                )
            } else {
                (Status::StagingError, clean_error_message(&stderr))
            }
        }
        Err(e) => {
            let error_message = clean_error_message(&e.to_string());
            (Status::StagingError, error_message)
        }
    }
}
