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
    worktree_status: Status,
    message: String,
    upstream: UpstreamSummary,
    failure: Option<GitFailure>,
}

struct FleetStatusEntry {
    repository: String,
    path: PathBuf,
    status: FleetStatus,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum FleetStatusKind {
    Healthy,
    NeedsWork,
    Failed,
}

impl FleetStatusKind {
    fn heading(self) -> &'static str {
        match self {
            Self::Healthy => "Healthy",
            Self::NeedsWork => "Needs Work",
            Self::Failed => "Failed",
        }
    }

    fn style(self) -> (&'static str, &'static str) {
        match self {
            Self::Healthy => (GREEN, "✓"),
            Self::NeedsWork => (YELLOW, "!"),
            Self::Failed => (RED, "!"),
        }
    }
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
            || self.upstream.needs_sync()
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
        self.worktree_status == Status::Dirty
    }

    fn kind(&self) -> FleetStatusKind {
        if self.failed() {
            FleetStatusKind::Failed
        } else if self.needs_work() {
            FleetStatusKind::NeedsWork
        } else {
            FleetStatusKind::Healthy
        }
    }

    fn next_action(&self, repo_path: &Path) -> Option<String> {
        if let Some(failure) = &self.failure {
            Some(failure.next_action(&format_relative_repo_path(&repo_path.to_string_lossy())))
        } else if self.failed() {
            Some("inspect the reported status failure".to_string())
        } else if self.dirty() {
            Some("commit or stash the local changes".to_string())
        } else if matches!(self.upstream, UpstreamSummary::NoRemote) {
            Some("add a remote or exclude this repository".to_string())
        } else if matches!(self.upstream, UpstreamSummary::NoUpstream) {
            Some("set an upstream branch or run `repos push --auto-upstream`".to_string())
        } else if self.upstream.is_diverged() {
            Some("run `repos sync` or resolve the divergence manually".to_string())
        } else if self.upstream.behind().is_some_and(|count| count > 0) {
            Some("run `repos pull`".to_string())
        } else if self.upstream.ahead().is_some_and(|count| count > 0) {
            Some("run `repos push`".to_string())
        } else {
            None
        }
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

/// Processes all repositories concurrently for status checking
async fn process_status_repositories(
    context: crate::core::ProcessingContext,
    filters: StatusFilters,
) -> usize {
    use crate::core::{acquire_semaphore_permit, create_progress_bar};
    use futures::stream::{FuturesUnordered, StreamExt};

    let mut futures = FuturesUnordered::new();
    let show_details = context.repositories.len() == 1;

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
    let start_time = context.start_time;
    for ((repo_name, repo_path), progress_bar) in
        context.repositories.iter().zip(repo_progress_bars)
    {
        let semaphore_clone = std::sync::Arc::clone(&context.semaphore);
        let repository = repo_name.clone();
        let path = repo_path.clone();

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore_clone).await;

            let status = get_fleet_status(&path, show_details).await;

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.status.symbol(),
                repository,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<12}   {}", status.status.text(), status.message));
            progress_bar.finish_and_clear();

            FleetStatusEntry {
                repository,
                path,
                status,
            }
        };

        futures.push(future);
    }

    // Wait for all repository operations to complete
    let mut entries = Vec::with_capacity(context.total_repos);
    while let Some(entry) = futures.next().await {
        entries.push(entry);
    }

    let failed = entries.iter().filter(|entry| entry.status.failed()).count();
    print_final_report(&generate_status_report(
        &entries,
        filters,
        start_time.elapsed(),
    ));
    failed
}

fn generate_status_report(
    entries: &[FleetStatusEntry],
    filters: StatusFilters,
    duration: std::time::Duration,
) -> String {
    let mut entries = entries.iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        compare_repository_locations(&left.path, &left.repository, &right.path, &right.repository)
    });

    let healthy = entries
        .iter()
        .filter(|entry| entry.status.kind() == FleetStatusKind::Healthy)
        .count();
    let needs_work = entries
        .iter()
        .filter(|entry| entry.status.kind() == FleetStatusKind::NeedsWork)
        .count();
    let failed = entries
        .iter()
        .filter(|entry| entry.status.kind() == FleetStatusKind::Failed)
        .count();
    let shown = entries
        .iter()
        .filter(|entry| entry.status.matches_filters(filters))
        .count();

    let mut lines = vec![
        format!("{BOLD_BLUE}repos status{RESET}"),
        format!(
            "{GREEN}✓{RESET} Completed in {:.1}s",
            duration.as_secs_f64()
        ),
        String::new(),
        format!("{BOLD_PURPLE}▌ Summary{RESET}"),
        format!("  {GREEN}✓{RESET} {:<16}{healthy}", "Healthy"),
    ];
    if needs_work > 0 {
        lines.push(format!(
            "  {YELLOW}!{RESET} {:<16}{needs_work}",
            "Needs work"
        ));
    }
    if failed > 0 {
        lines.push(format!("  {RED}!{RESET} {:<16}{failed}", "Failed"));
    }
    if !filters.is_empty() {
        lines.push(format!("  {DIM}·{RESET} {:<16}{shown}", "Shown"));
    }
    lines.push(format!(
        "  {DIM}·{RESET} {:<16}{}",
        "Checked",
        entries.len()
    ));

    append_status_section(&mut lines, &entries, filters, FleetStatusKind::Failed);
    append_status_section(&mut lines, &entries, filters, FleetStatusKind::NeedsWork);
    append_status_section(&mut lines, &entries, filters, FleetStatusKind::Healthy);

    if shown == 0 {
        lines.push(String::new());
        lines.push(format!("{BOLD_PURPLE}▌ Repositories{RESET}"));
        lines.push(format!(
            "  {DIM}No repositories matched the requested status filters.{RESET}"
        ));
    }

    lines.join("\n")
}

fn append_status_section(
    lines: &mut Vec<String>,
    entries: &[&FleetStatusEntry],
    filters: StatusFilters,
    kind: FleetStatusKind,
) {
    let matching = entries
        .iter()
        .filter(|entry| entry.status.kind() == kind && entry.status.matches_filters(filters))
        .copied()
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return;
    }

    lines.push(String::new());
    lines.push(format!("{BOLD_PURPLE}▌ {}{RESET}", kind.heading()));
    let (color, marker) = kind.style();
    for entry in matching {
        lines.push(format!(
            "  {color}{marker}{RESET} {:24} {}",
            truncate_text(&entry.repository, 24),
            entry.status.message
        ));
        lines.push(format!(
            "    {DIM}↳ path: {}{RESET}",
            format_relative_repo_path(&entry.path.to_string_lossy())
        ));
        if let Some(remote) = entry
            .status
            .failure
            .as_ref()
            .and_then(|failure| failure.remote.as_ref())
        {
            lines.push(format!("    {DIM}↳ remote: {}{RESET}", remote.display()));
        }
        if let Some(next) = entry.status.next_action(&entry.path) {
            lines.push(format!("    {DIM}↳ next: {next}{RESET}"));
        }
    }
}

async fn get_fleet_status(repo_path: &std::path::Path, show_details: bool) -> FleetStatus {
    let initial_state =
        match crate::git::worktree::inspect_refreshed_repository_state(repo_path).await {
            Ok(state) => state,
            Err(error) => {
                return FleetStatus {
                    status: Status::StagingError,
                    worktree_status: Status::StagingError,
                    message: format!("status failed: {}", clean_error_message(&error.to_string())),
                    upstream: UpstreamSummary::Unknown,
                    failure: None,
                }
            }
        };

    let status_result = get_staging_status(repo_path).await;
    let (working_status, mut parts, details) = match status_result {
        Ok((stdout, _)) => summarize_worktree(&stdout, show_details),
        Err(e) => {
            return FleetStatus {
                status: Status::StagingError,
                worktree_status: Status::StagingError,
                message: format!("status failed: {}", clean_error_message(&e.to_string())),
                upstream: UpstreamSummary::Unknown,
                failure: None,
            };
        }
    };

    let branch = match initial_state.head() {
        crate::git::worktree::HeadState::Branch(branch) => branch.clone(),
        crate::git::worktree::HeadState::Detached => "HEAD".to_string(),
        crate::git::worktree::HeadState::Unborn => "unborn".to_string(),
        crate::git::worktree::HeadState::Unknown => "unknown".to_string(),
    };
    parts.insert(0, format!("branch {branch}"));

    let refresh =
        crate::git::operations::fetch_and_analyze_for_pull_with_state(repo_path, initial_state)
            .await;
    if refresh.status == Status::Error {
        let failure = refresh.failure;
        let reason = failure
            .as_ref()
            .map_or_else(|| clean_error_message(&refresh.message), GitFailure::reason);
        parts.push(reason);
        let mut message = parts.join(" | ");
        if !details.is_empty() {
            message.push('\n');
            message.push_str(&details.join("\n"));
        }
        return FleetStatus {
            status: Status::Error,
            worktree_status: working_status,
            message,
            upstream: UpstreamSummary::Unknown,
            failure,
        };
    }

    let upstream = match refresh.status {
        Status::NoRemote => UpstreamSummary::NoRemote,
        Status::NoUpstream => UpstreamSummary::NoUpstream,
        Status::Skip => UpstreamSummary::Unknown,
        _ => UpstreamSummary::from_counts(
            refresh.upstream_name.as_deref().unwrap_or("upstream"),
            refresh.ahead_count,
            refresh.behind_count,
        ),
    };
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
        worktree_status: working_status,
        message,
        upstream,
        failure: None,
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
    fn from_counts(upstream: &str, ahead: u32, behind: u32) -> Self {
        let message = if ahead > 0 && behind > 0 {
            format!("diverged ({ahead} ahead, {behind} behind)")
        } else if ahead > 0 {
            format!("ahead {ahead}")
        } else if behind > 0 {
            format!("behind {behind}")
        } else {
            format!("synced with {upstream}")
        };
        Self::Remote {
            message,
            ahead,
            behind,
        }
    }

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

    fn needs_sync(&self) -> bool {
        matches!(self, UpstreamSummary::Remote { ahead, behind, .. } if *ahead > 0 || *behind > 0)
    }

    fn ahead(&self) -> Option<u32> {
        match self {
            UpstreamSummary::Remote { ahead, .. } => Some(*ahead),
            _ => None,
        }
    }

    fn behind(&self) -> Option<u32> {
        match self {
            UpstreamSummary::Remote { behind, .. } => Some(*behind),
            _ => None,
        }
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

#[cfg(test)]
mod tests {
    use super::{
        filter_status_repositories, format_status_details, generate_status_report,
        summarize_worktree, FleetStatus, FleetStatusEntry, StatusFilters, UpstreamSummary,
    };
    use crate::git::Status;
    use std::path::PathBuf;
    use std::time::Duration;

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

    #[test]
    fn final_status_report_is_attributable_exclusive_and_filter_aware() {
        let entries = vec![
            FleetStatusEntry {
                repository: "healthy".to_string(),
                path: PathBuf::from("healthy"),
                status: FleetStatus {
                    status: Status::Synced,
                    worktree_status: Status::Synced,
                    message: "branch main | clean | synced with origin/main".to_string(),
                    upstream: UpstreamSummary::Remote {
                        message: "synced with origin/main".to_string(),
                        ahead: 0,
                        behind: 0,
                    },
                    failure: None,
                },
            },
            FleetStatusEntry {
                repository: "dirty".to_string(),
                path: PathBuf::from("dirty"),
                status: FleetStatus {
                    status: Status::Dirty,
                    worktree_status: Status::Dirty,
                    message: "branch main | 2 unstaged".to_string(),
                    upstream: UpstreamSummary::NoUpstream,
                    failure: None,
                },
            },
            FleetStatusEntry {
                repository: "broken".to_string(),
                path: PathBuf::from("broken"),
                status: FleetStatus {
                    status: Status::StagingError,
                    worktree_status: Status::StagingError,
                    message: "status failed: permission denied".to_string(),
                    upstream: UpstreamSummary::Unknown,
                    failure: None,
                },
            },
        ];

        let report =
            generate_status_report(&entries, StatusFilters::default(), Duration::from_secs(2));
        assert!(report.contains("repos status"));
        assert!(report.contains("Healthy         1"));
        assert!(report.contains("Needs work      1"));
        assert!(report.contains("Failed          1"));
        assert!(report.contains("Checked         3"));
        assert!(report.contains("path: ./healthy"));
        assert!(report.contains("path: ./dirty"));
        assert!(report.contains("next: commit or stash"));
        assert!(report.contains("path: ./broken"));
        assert!(report.contains("next: inspect the reported status failure"));

        let filtered = generate_status_report(
            &entries,
            StatusFilters {
                dirty: true,
                ..StatusFilters::default()
            },
            Duration::ZERO,
        );
        assert!(filtered.contains("Shown           1"));
        assert!(filtered.contains("path: ./dirty"));
        assert!(!filtered.contains("path: ./healthy"));
        assert!(!filtered.contains("path: ./broken"));
    }

    #[test]
    fn ahead_and_behind_repositories_need_action() {
        for (ahead, behind, next) in [
            (1, 0, "run `repos push`"),
            (0, 2, "run `repos pull`"),
            (1, 2, "run `repos sync` or resolve the divergence manually"),
        ] {
            let status = FleetStatus {
                status: Status::Synced,
                worktree_status: Status::Synced,
                message: String::new(),
                upstream: UpstreamSummary::Remote {
                    message: String::new(),
                    ahead,
                    behind,
                },
                failure: None,
            };

            assert!(status.needs_work());
            assert_eq!(
                status.next_action(&PathBuf::from("repo")).as_deref(),
                Some(next)
            );
        }
    }

    #[test]
    fn dirty_filter_preserves_worktree_state_when_remote_refresh_fails() {
        let status = FleetStatus {
            status: Status::Error,
            worktree_status: Status::Dirty,
            message: "branch main | 1 unstaged | network error during fetch".to_string(),
            upstream: UpstreamSummary::Unknown,
            failure: None,
        };

        assert!(status.dirty());
        assert!(status.failed());
        assert!(status.matches_filters(StatusFilters {
            dirty: true,
            ..StatusFilters::default()
        }));
    }
}
