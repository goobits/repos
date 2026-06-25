//! Repository staging command implementation
//!
//! This module handles staging operations across multiple repositories:
//! - Stage files matching patterns
//! - Unstage files matching patterns
//! - Show staging status across repositories
//! - Commit staged changes across repositories

use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::core::{
    create_processing_context, init_command, set_terminal_title, set_terminal_title_and_flush,
    GIT_CONCURRENT_CAP, NO_REPOS_MESSAGE,
};
use crate::git::{
    commit_changes, get_staging_status, has_staged_changes, is_detached_head, stage_files,
    unstage_files, Status,
};

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";
const STAGING_MESSAGE: &str = "staging...";
const UNSTAGING_MESSAGE: &str = "unstaging...";
const STATUS_MESSAGE: &str = "checking status...";
const COMMITTING_MESSAGE: &str = "committing...";

#[derive(Clone, Copy, Debug, Default)]
pub struct StatusFilters {
    pub needs_work: bool,
    pub dirty: bool,
    pub no_remote: bool,
    pub no_upstream: bool,
    pub failed: bool,
    pub skipped: bool,
}

impl StatusFilters {
    fn is_empty(self) -> bool {
        !self.needs_work
            && !self.dirty
            && !self.no_remote
            && !self.no_upstream
            && !self.failed
            && !self.skipped
    }
}

struct FleetStatus {
    status: Status,
    message: String,
    upstream: UpstreamSummary,
}

impl FleetStatus {
    fn matches_filters(&self, filters: StatusFilters) -> bool {
        if filters.is_empty() {
            return true;
        }

        (filters.needs_work && self.needs_work())
            || (filters.dirty && self.dirty())
            || (filters.no_remote && matches!(self.upstream, UpstreamSummary::NoRemote))
            || (filters.no_upstream && matches!(self.upstream, UpstreamSummary::NoUpstream))
            || (filters.failed && self.failed())
            || (filters.skipped && self.skipped_for_push())
    }

    fn needs_work(&self) -> bool {
        self.dirty()
            || matches!(
                self.upstream,
                UpstreamSummary::NoRemote | UpstreamSummary::NoUpstream
            )
            || self.upstream.is_diverged()
            || self.failed()
    }

    fn failed(&self) -> bool {
        matches!(
            self.status,
            Status::Error | Status::StagingError | Status::CommitError | Status::PullError
        )
    }

    fn skipped_for_push(&self) -> bool {
        !self.failed()
            && !self.upstream.is_diverged()
            && (matches!(
                self.upstream,
                UpstreamSummary::NoRemote | UpstreamSummary::NoUpstream
            ) || self.upstream.ahead() == Some(0))
    }

    fn dirty(&self) -> bool {
        self.status == Status::Dirty
    }
}

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

    process_staging_repositories(context, pattern, true).await;
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

    process_staging_repositories(context, pattern, false).await;
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
    process_status_repositories(context, filters).await;

    // Set terminal title to green checkbox to indicate completion
    set_terminal_title_and_flush("✅ repos status");

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
) {
    use crate::core::{acquire_semaphore_permit, acquire_stats_lock, create_progress_bar};
    use futures::stream::{FuturesUnordered, StreamExt};

    let mut futures = FuturesUnordered::new();

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
            let stats_guard = acquire_stats_lock(&stats_clone);
            let repo_path_str = repo_path.to_string_lossy();
            stats_guard.update(
                repo_name,
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
        println!("{detailed_summary}");
        println!("{}", "━".repeat(70));
    }

    // Add final spacing
    println!();
}

/// Processes all repositories concurrently for status checking
async fn process_status_repositories(
    context: crate::core::ProcessingContext,
    filters: StatusFilters,
) {
    use crate::core::{acquire_semaphore_permit, create_progress_bar};
    use futures::stream::{FuturesUnordered, StreamExt};

    if context.repositories.len() == 1 {
        if let Some((repo_name, repo_path)) = context.repositories.first() {
            let status = get_fleet_status(repo_path, true).await;
            if status.matches_filters(filters) {
                println!(
                    "{} {:width$} {:<12}   {}",
                    status.status.symbol(),
                    repo_name,
                    status.status.text(),
                    status.message,
                    width = context.max_name_length
                );
            } else {
                println!("No repositories matched the requested status filters.");
            }
            println!();
        }
        return;
    }

    let mut futures = FuturesUnordered::new();

    // First, create all repository progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in context.repositories.iter() {
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
        context.repositories.iter().zip(repo_progress_bars)
    {
        let semaphore_clone = std::sync::Arc::clone(&context.semaphore);

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore_clone).await;

            let status = get_fleet_status(repo_path, false).await;
            if !status.matches_filters(filters) {
                progress_bar.finish_and_clear();
                return;
            }

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.status.symbol(),
                repo_name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<12}   {}", status.status.text(), status.message));
            progress_bar.finish();
        };

        futures.push(future);
    }

    // Wait for all repository operations to complete
    while futures.next().await.is_some() {}

    // Add final spacing
    println!();
}

async fn get_fleet_status(repo_path: &std::path::Path, show_details: bool) -> FleetStatus {
    use crate::git::operations::run_git;

    let status_result = get_staging_status(repo_path).await;
    let (working_status, mut parts, details) = match status_result {
        Ok((stdout, _)) => summarize_worktree(&stdout, show_details),
        Err(e) => {
            return FleetStatus {
                status: Status::StagingError,
                message: format!("status failed: {e}"),
                upstream: UpstreamSummary::Unknown,
            };
        }
    };

    let branch = match run_git(repo_path, &["rev-parse", "--abbrev-ref", "HEAD"]).await {
        Ok((true, branch, _)) => branch,
        _ => "unknown".to_string(),
    };
    parts.insert(0, format!("branch {branch}"));

    let upstream = summarize_upstream(repo_path).await;
    if let Some(summary) = upstream.message() {
        parts.push(summary.to_string());
    }

    let mut message = parts.join(" | ");
    if !details.is_empty() {
        message.push('\n');
        message.push_str(&details.join("\n"));
    }

    FleetStatus {
        status: working_status,
        message,
        upstream,
    }
}

fn summarize_worktree(stdout: &str, show_details: bool) -> (Status, Vec<String>, Vec<String>) {
    if stdout.trim().is_empty() {
        return (Status::Synced, vec!["clean".to_string()], Vec::new());
    }

    let lines: Vec<&str> = stdout.lines().filter(|line| !line.is_empty()).collect();
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
            chars.len() >= 2 && chars[1] != ' ' && !line.starts_with("??")
        })
        .count();
    let untracked_count = lines.iter().filter(|line| line.starts_with("??")).count();

    let mut parts = Vec::new();
    if staged_count > 0 {
        parts.push(format!("{staged_count} staged"));
    }
    if unstaged_count > 0 {
        parts.push(format!("{unstaged_count} unstaged"));
    }
    if untracked_count > 0 {
        parts.push(format!("{untracked_count} untracked"));
    }

    if parts.is_empty() {
        (Status::Synced, vec!["clean".to_string()], Vec::new())
    } else {
        let details = if show_details {
            format_status_details(&lines)
        } else {
            Vec::new()
        };
        (Status::Dirty, parts, details)
    }
}

fn format_status_details(lines: &[&str]) -> Vec<String> {
    const MAX_FILES: usize = 20;

    lines
        .iter()
        .take(MAX_FILES)
        .map(|line| {
            let status = line.get(..2).unwrap_or(line);
            let path = line.get(3..).unwrap_or("").trim();
            format!("    {} {}", status_detail_label(status), path)
        })
        .chain(
            (lines.len() > MAX_FILES)
                .then(|| format!("    · ... and {} more", lines.len() - MAX_FILES)),
        )
        .collect()
}

fn status_detail_label(status: &str) -> &'static str {
    if status == "??" {
        "· untracked"
    } else if status.chars().next().is_some_and(|state| state != ' ') {
        "✓ staged  "
    } else if status.chars().nth(1).is_some_and(|state| state != ' ') {
        "! unstaged"
    } else {
        "· changed "
    }
}

enum UpstreamSummary {
    Remote {
        message: String,
        ahead: u32,
        behind: u32,
    },
    NoRemote,
    NoUpstream,
    Unknown,
}

impl UpstreamSummary {
    fn message(&self) -> Option<&str> {
        match self {
            UpstreamSummary::Remote { message, .. } => Some(message),
            UpstreamSummary::NoRemote => Some("no remote"),
            UpstreamSummary::NoUpstream => Some("no upstream"),
            UpstreamSummary::Unknown => None,
        }
    }

    fn is_diverged(&self) -> bool {
        matches!(self, UpstreamSummary::Remote { ahead, behind, .. } if *ahead > 0 && *behind > 0)
    }

    fn ahead(&self) -> Option<u32> {
        match self {
            UpstreamSummary::Remote { ahead, .. } => Some(*ahead),
            _ => None,
        }
    }
}

async fn summarize_upstream(repo_path: &std::path::Path) -> UpstreamSummary {
    use crate::git::operations::run_git;

    let upstream = run_git(repo_path, &["rev-parse", "--abbrev-ref", "@{upstream}"])
        .await
        .ok();
    let Some(upstream) = upstream else {
        return summarize_missing_upstream(repo_path).await;
    };
    if !upstream.0 {
        return summarize_missing_upstream(repo_path).await;
    }

    let ahead = run_git(repo_path, &["rev-list", "--count", "HEAD", "^@{upstream}"])
        .await
        .ok()
        .and_then(|(success, count, _)| {
            if success {
                count.parse::<u32>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    let behind = run_git(repo_path, &["rev-list", "--count", "@{upstream}", "^HEAD"])
        .await
        .ok()
        .and_then(|(success, count, _)| {
            if success {
                count.parse::<u32>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    let remote = if ahead > 0 && behind > 0 {
        format!("diverged ({ahead} ahead, {behind} behind)")
    } else if ahead > 0 {
        format!("ahead {ahead}")
    } else if behind > 0 {
        format!("behind {behind}")
    } else {
        format!("synced with {}", upstream.1)
    };

    UpstreamSummary::Remote {
        message: remote,
        ahead,
        behind,
    }
}

async fn summarize_missing_upstream(repo_path: &std::path::Path) -> UpstreamSummary {
    use crate::git::operations::run_git;

    match run_git(repo_path, &["remote"]).await {
        Ok((true, remotes, _)) if !remotes.trim().is_empty() => UpstreamSummary::NoUpstream,
        _ => UpstreamSummary::NoRemote,
    }
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

    process_commit_repositories(context, message, include_empty).await;
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
        context.repositories.iter().zip(repo_progress_bars)
    {
        let stats_clone = std::sync::Arc::clone(&context.statistics);
        let semaphore_clone = std::sync::Arc::clone(&context.semaphore);
        let footer_clone = footer_pb.clone();
        let message_clone = message.clone();

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore_clone).await;

            let (status, message) =
                perform_commit_operation(repo_path, &message_clone, include_empty).await;

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                repo_name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<12}   {}", status.text(), message));
            progress_bar.finish();

            // Update statistics based on operation result
            let stats_guard = acquire_stats_lock(&stats_clone);
            let repo_path_str = repo_path.to_string_lossy();
            stats_guard.update(
                repo_name,
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
        println!("{detailed_summary}");
        println!("{}", "━".repeat(70));
    }

    // Add final spacing
    println!();
}

/// Performs a staging operation on a single repository
async fn perform_staging_operation(repo_path: &std::path::Path, pattern: &str) -> (Status, String) {
    use crate::core::clean_error_message;

    match stage_files(repo_path, pattern).await {
        Ok((true, _, _)) => (Status::Staged, format!("staged {pattern}")),
        Ok((false, _, stderr)) => {
            let error_message = clean_error_message(&stderr);
            if error_message.contains("pathspec") && error_message.contains("did not match") {
                (Status::NoChanges, format!("no files match {pattern}"))
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
            let error_message = clean_error_message(&stderr);
            if error_message.contains("pathspec") && error_message.contains("did not match") {
                (
                    Status::NoChanges,
                    format!("no staged files match {pattern}"),
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

#[cfg(test)]
mod tests {
    use super::{filter_status_repositories, format_status_details, summarize_worktree};
    use crate::git::Status;
    use std::path::PathBuf;

    #[test]
    fn filters_status_repositories_by_name() {
        let repos = vec![
            ("frontdesk".to_string(), PathBuf::from("./frontdesk")),
            ("tunajack.com".to_string(), PathBuf::from("./tunajack.com")),
        ];

        let filtered = filter_status_repositories(repos, &["tunajack.com".to_string()]);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, "tunajack.com");
    }

    #[test]
    fn filters_status_repositories_by_relative_path() {
        let repos = vec![
            ("logger".to_string(), PathBuf::from("./packages/logger")),
            ("frontdesk".to_string(), PathBuf::from("./frontdesk")),
        ];

        let filtered = filter_status_repositories(repos, &["packages/logger".to_string()]);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, "logger");
    }

    #[test]
    fn summarizes_worktree_without_counting_untracked_as_unstaged() {
        let (status, parts, details) = summarize_worktree(" M README.md\n?? notes.txt\n", false);

        assert_eq!(status, Status::Dirty);
        assert_eq!(parts, vec!["1 unstaged", "1 untracked"]);
        assert!(details.is_empty());
    }

    #[test]
    fn formats_single_repo_status_details() {
        let details =
            format_status_details(&["M  staged.txt", " M unstaged.txt", "?? new-file.txt"]);

        assert_eq!(
            details,
            vec![
                "    ✓ staged   staged.txt",
                "    ! unstaged unstaged.txt",
                "    · untracked new-file.txt",
            ]
        );
    }
}
