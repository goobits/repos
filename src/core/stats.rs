//! Statistics tracking for repository operations

mod format;
mod report;

pub(super) use format::get_repo_changes;
pub(crate) use format::{clean_error_message, format_relative_repo_path, truncate_text};
use format::{parse_commit_count, pluralize};
use report::*;

use super::attention::{append_project_attention_section, AttentionKind, ProjectAttention};
use super::report::RepositoryOutcome;
use crate::git::failure::{GitFailure, GitOperationResult};
use crate::git::Status;
use crate::utils::compare_repository_locations;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

pub(super) const RESET: &str = "\x1b[0m";
pub(super) const BOLD_BLUE: &str = "\x1b[1;38;5;75m";
pub(super) const BOLD_PURPLE: &str = "\x1b[1;38;5;141m";
pub(super) const GREEN: &str = "\x1b[1;38;5;114m";
pub(super) const YELLOW: &str = "\x1b[1;38;5;221m";
pub(super) const RED: &str = "\x1b[1;38;5;203m";
pub(super) const DIM: &str = "\x1b[2m";

#[derive(Clone, Copy)]
enum Transfer {
    Fetch,
    Push,
    Pull,
}

#[derive(Clone, Copy)]
struct TransferTotals {
    transferred_repos: u64,
    transferred_commits: u64,
    up_to_date: u64,
    errors: u64,
    skipped: u64,
    follow_up: usize,
    checked: u64,
}

impl Transfer {
    fn command(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Push => "push",
            Self::Pull => "pull",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Fetch => "Fetched",
            Self::Push => "Pushed",
            Self::Pull => "Pulled",
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Self::Fetch => "fetched",
            Self::Push => "pushed",
            Self::Pull => "pulled",
        }
    }

    fn marker(self) -> &'static str {
        match self {
            Self::Fetch => "↻",
            Self::Push => "↑",
            Self::Pull => "↓",
        }
    }

    fn unit(self, count: u64) -> &'static str {
        match self {
            Self::Fetch => pluralize(count, "ref", "refs"),
            Self::Push | Self::Pull => pluralize(count, "commit", "commits"),
        }
    }

    fn no_upstream_action(self) -> &'static str {
        match self {
            Self::Fetch => "configure a fetch remote or skip",
            Self::Push => "repos push --auto-upstream",
            Self::Pull => "set upstream or skip",
        }
    }
}

/// Statistics for tracking repository synchronization results
///
/// Uses atomic counters for lock-free reads and writes of simple counters,
/// while complex data structures (vectors) remain behind a Mutex.
#[derive(Debug)]
pub struct SyncStatistics {
    // Atomic counters for lock-free access
    pub synced_repos: AtomicU64,
    pub pushed_repos: AtomicU64,
    pub total_commits_pushed: AtomicU64,
    pub pulled_repos: AtomicU64,
    pub total_commits_pulled: AtomicU64,
    pub fetched_repos: AtomicU64,
    pub total_refs_fetched: AtomicU64,
    pub skipped_repos: AtomicU64,
    pub error_repos: AtomicU64,
    pub uncommitted_count: AtomicU64,
    // Complex data behind mutex
    pub failed_repos: Mutex<Vec<(String, String, String)>>, // (repo_name, repo_path, error_message)
    pub uncommitted_repos: Mutex<Vec<(String, String)>>,    // (repo_name, repo_path)
    pub pushed_repo_details: Mutex<Vec<(String, String, u64)>>, // (repo_name, repo_path, commits)
    pub pulled_repo_details: Mutex<Vec<(String, String, u64)>>, // (repo_name, repo_path, commits)
    pub fetched_repo_details: Mutex<Vec<(String, String, u64)>>, // (repo_name, repo_path, refs)
    pub(crate) operation_outcomes: Mutex<Vec<RepositoryOutcome>>,
    pub(crate) git_failures: Mutex<HashMap<(String, String), GitFailure>>,
}

impl Default for SyncStatistics {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncStatistics {
    /// Creates a new statistics tracker with all counters initialized to zero
    #[must_use]
    pub fn new() -> Self {
        Self {
            synced_repos: AtomicU64::new(0),
            pushed_repos: AtomicU64::new(0),
            total_commits_pushed: AtomicU64::new(0),
            pulled_repos: AtomicU64::new(0),
            total_commits_pulled: AtomicU64::new(0),
            fetched_repos: AtomicU64::new(0),
            total_refs_fetched: AtomicU64::new(0),
            skipped_repos: AtomicU64::new(0),
            error_repos: AtomicU64::new(0),
            uncommitted_count: AtomicU64::new(0),
            failed_repos: Mutex::new(Vec::new()),
            uncommitted_repos: Mutex::new(Vec::new()),
            pushed_repo_details: Mutex::new(Vec::new()),
            pulled_repo_details: Mutex::new(Vec::new()),
            fetched_repo_details: Mutex::new(Vec::new()),
            operation_outcomes: Mutex::new(Vec::new()),
            git_failures: Mutex::new(HashMap::new()),
        }
    }

    /// Updates statistics based on the synchronization result
    pub fn update(
        &self,
        repo_name: &str,
        repo_path: &str,
        status: &Status,
        message: &str,
        has_uncommitted: bool,
    ) {
        self.update_with_transfer_count(
            repo_name,
            repo_path,
            status,
            message,
            has_uncommitted,
            None,
        );
    }

    fn update_with_transfer_count(
        &self,
        repo_name: &str,
        repo_path: &str,
        status: &Status,
        message: &str,
        has_uncommitted: bool,
        transfer_count: Option<u64>,
    ) {
        if let Ok(mut outcomes) = self.operation_outcomes.lock() {
            outcomes.push(RepositoryOutcome {
                repository: repo_name.to_string(),
                path: repo_path.to_string(),
                status: *status,
                message: message.to_string(),
                has_uncommitted,
            });
        } else {
            eprintln!("Warning: Failed to record operation result for repo: {repo_name}");
        }

        match status {
            Status::Pushed => {
                self.synced_repos.fetch_add(1, Ordering::Relaxed);
                self.pushed_repos.fetch_add(1, Ordering::Relaxed);
                let commits = transfer_count
                    .or_else(|| parse_commit_count(message))
                    .unwrap_or(0);
                if commits > 0 {
                    self.total_commits_pushed
                        .fetch_add(commits, Ordering::Relaxed);
                }
                if let Ok(mut guard) = self.pushed_repo_details.lock() {
                    guard.push((repo_name.to_string(), repo_path.to_string(), commits));
                } else {
                    eprintln!("Warning: Failed to record pushed repo: {repo_name}");
                }
            }
            Status::Pulled => {
                self.synced_repos.fetch_add(1, Ordering::Relaxed);
                self.pulled_repos.fetch_add(1, Ordering::Relaxed);
                let commits = transfer_count
                    .or_else(|| parse_commit_count(message))
                    .unwrap_or(0);
                if commits > 0 {
                    self.total_commits_pulled
                        .fetch_add(commits, Ordering::Relaxed);
                }
                if let Ok(mut guard) = self.pulled_repo_details.lock() {
                    guard.push((repo_name.to_string(), repo_path.to_string(), commits));
                } else {
                    eprintln!("Warning: Failed to record pulled repo: {repo_name}");
                }
            }
            Status::Fetched => {
                self.synced_repos.fetch_add(1, Ordering::Relaxed);
                self.fetched_repos.fetch_add(1, Ordering::Relaxed);
                let refs = transfer_count
                    .or_else(|| parse_commit_count(message))
                    .unwrap_or(0);
                if refs > 0 {
                    self.total_refs_fetched.fetch_add(refs, Ordering::Relaxed);
                }
                if let Ok(mut guard) = self.fetched_repo_details.lock() {
                    guard.push((repo_name.to_string(), repo_path.to_string(), refs));
                } else {
                    eprintln!("Warning: Failed to record fetched repo: {repo_name}");
                }
            }
            Status::Synced
            | Status::ConfigSynced
            | Status::ConfigUpdated
            | Status::Staged
            | Status::Unstaged
            | Status::Committed => {
                self.synced_repos.fetch_add(1, Ordering::Relaxed);
            }
            Status::Skip
            | Status::ConfigSkipped
            | Status::NoChanges
            | Status::Dirty
            | Status::NoUpstream
            | Status::NoRemote => {
                self.skipped_repos.fetch_add(1, Ordering::Relaxed);
            }
            Status::Error
            | Status::ConfigError
            | Status::StagingError
            | Status::CommitError
            | Status::PullError => {
                self.error_repos.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut guard) = self.failed_repos.lock() {
                    guard.push((
                        repo_name.to_string(),
                        repo_path.to_string(),
                        message.to_string(),
                    ));
                } else {
                    eprintln!("Warning: Failed to record error for repo: {repo_name}");
                }
            }
        }

        // Only track uncommitted changes for non-failed repos
        if has_uncommitted
            && !matches!(
                status,
                Status::Error
                    | Status::ConfigError
                    | Status::StagingError
                    | Status::CommitError
                    | Status::PullError
            )
        {
            if let Ok(mut uncommitted) = self.uncommitted_repos.lock() {
                if !uncommitted.iter().any(|(name, _)| name == repo_name) {
                    self.uncommitted_count.fetch_add(1, Ordering::Relaxed);
                    uncommitted.push((repo_name.to_string(), repo_path.to_string()));
                }
            } else {
                eprintln!("Warning: Failed to record uncommitted changes for repo: {repo_name}");
            }
        }
    }

    /// Updates transfer statistics from typed operation data and retains
    /// structured Git context for actionable reports.
    pub(crate) fn update_operation(
        &self,
        repo_name: &str,
        repo_path: &str,
        result: &GitOperationResult,
    ) {
        self.update_with_transfer_count(
            repo_name,
            repo_path,
            &result.status,
            &result.message,
            result.has_uncommitted,
            Some(result.transferred),
        );

        if let Some(failure) = &result.failure {
            if let Ok(mut failures) = self.git_failures.lock() {
                failures.insert(
                    (repo_name.to_string(), repo_path.to_string()),
                    failure.clone(),
                );
            } else {
                eprintln!("Warning: Failed to retain Git failure context for repo: {repo_name}");
            }
        }
    }

    /// Generates a push-specific completion summary.
    pub fn generate_push_summary(&self, duration: Duration) -> String {
        self.generate_transfer_summary(Transfer::Push, duration)
    }

    /// Generates a fetch-specific completion summary.
    pub fn generate_fetch_summary(&self, duration: Duration) -> String {
        self.generate_transfer_summary(Transfer::Fetch, duration)
    }

    /// Generates a pull/sync-specific completion summary.
    pub fn generate_pull_summary(&self, duration: Duration) -> String {
        self.generate_transfer_summary(Transfer::Pull, duration)
    }

    fn generate_transfer_summary(&self, transfer: Transfer, duration: Duration) -> String {
        let duration_secs = duration.as_secs_f64();
        let (transferred_repos, transferred_commits) = self.transfer_counts(transfer);
        let up_to_date = self.up_to_date_count(transfer);
        let verb = transfer.verb();
        let errors = self.error_repos.load(Ordering::Relaxed);
        let skipped = self.skipped_repos.load(Ordering::Relaxed);

        let unit = transfer.unit(transferred_commits);
        format!(
            "✅ Completed in {duration_secs:.1}s • {up_to_date} up to date • {transferred_repos} {verb} ({transferred_commits} {unit}) • {errors} failed • {skipped} skipped"
        )
    }

    /// Generates a compact live push footer.
    pub fn generate_push_live_summary(&self, total_repos: usize) -> String {
        self.generate_transfer_live_summary(Transfer::Push, total_repos)
    }

    /// Generates a compact live fetch footer.
    pub fn generate_fetch_live_summary(&self, total_repos: usize) -> String {
        self.generate_transfer_live_summary(Transfer::Fetch, total_repos)
    }

    /// Generates a compact live pull footer.
    pub fn generate_pull_live_summary(&self, total_repos: usize) -> String {
        self.generate_transfer_live_summary(Transfer::Pull, total_repos)
    }

    fn generate_transfer_live_summary(&self, transfer: Transfer, total_repos: usize) -> String {
        let (transferred_repos, transferred_commits) = self.transfer_counts(transfer);
        let up_to_date = self.up_to_date_count(transfer);
        let marker = transfer.marker();
        let verb = transfer.verb();
        let errors = self.error_repos.load(Ordering::Relaxed);
        let skipped = self.skipped_repos.load(Ordering::Relaxed);
        let follow_up = self.transfer_follow_up_count();
        let unit = transfer.unit(transferred_commits);
        let processed = up_to_date
            .saturating_add(transferred_repos)
            .saturating_add(errors)
            .saturating_add(skipped);
        let remaining = (total_repos as u64).saturating_sub(processed);

        format!(
            "  {GREEN}✓{RESET} {up_to_date} up to date   {GREEN}{marker}{RESET} {transferred_repos} {verb} / {transferred_commits} {unit}   {RED}!{RESET} {errors} failed   {DIM}·{RESET} {skipped} skipped   {YELLOW}~{RESET} {follow_up} follow-up\n  {DIM}↳ scanning {remaining} remaining{RESET}",
        )
    }

    /// Generates the final push report without repeating the live footer details.
    pub fn generate_push_report(&self, duration: Duration, show_changes: bool) -> String {
        self.generate_push_report_with_follow_up(duration, show_changes, 0, &[])
    }

    /// Generates the final fetch report.
    pub fn generate_fetch_report(&self, duration: Duration) -> String {
        self.generate_transfer_report_with_follow_up(Transfer::Fetch, duration, false, 0, &[])
    }

    /// Generates the final pull report without repeating the live footer details.
    pub fn generate_pull_report(&self, duration: Duration, show_changes: bool) -> String {
        self.generate_pull_report_with_follow_up(duration, show_changes, 0, &[])
    }

    /// Generates the final push report with additional actionable work lines.
    pub fn generate_push_report_with_follow_up(
        &self,
        duration: Duration,
        show_changes: bool,
        extra_follow_up_count: usize,
        extra_follow_up_lines: &[String],
    ) -> String {
        self.generate_transfer_report_with_follow_up(
            Transfer::Push,
            duration,
            show_changes,
            extra_follow_up_count,
            extra_follow_up_lines,
        )
    }

    /// Generates the final pull report with additional actionable work lines.
    pub fn generate_pull_report_with_follow_up(
        &self,
        duration: Duration,
        show_changes: bool,
        extra_follow_up_count: usize,
        extra_follow_up_lines: &[String],
    ) -> String {
        self.generate_transfer_report_with_follow_up(
            Transfer::Pull,
            duration,
            show_changes,
            extra_follow_up_count,
            extra_follow_up_lines,
        )
    }

    fn generate_transfer_report_with_follow_up(
        &self,
        transfer: Transfer,
        duration: Duration,
        show_changes: bool,
        extra_follow_up_count: usize,
        extra_follow_up_lines: &[String],
    ) -> String {
        let duration_secs = duration.as_secs_f64();
        let (transferred_repos, transferred_commits) = self.transfer_counts(transfer);
        let up_to_date = self.up_to_date_count(transfer);
        let skipped = self.skipped_repos.load(Ordering::Relaxed);
        let errors = self.error_repos.load(Ordering::Relaxed);
        let checked = up_to_date
            .saturating_add(transferred_repos)
            .saturating_add(skipped)
            .saturating_add(errors);

        let mut transferred_details = match transfer {
            Transfer::Fetch => clone_vec(&self.fetched_repo_details, "fetched_repo_details"),
            Transfer::Push => clone_vec(&self.pushed_repo_details, "pushed_repo_details"),
            Transfer::Pull => clone_vec(&self.pulled_repo_details, "pulled_repo_details"),
        };
        let mut failed_repos = clone_vec(&self.failed_repos, "failed_repos");
        let mut outcomes = clone_vec(&self.operation_outcomes, "operation_outcomes");
        let git_failures = clone_failure_map(&self.git_failures);

        transferred_details.sort_by(|left, right| {
            compare_repository_locations(&left.1, &left.0, &right.1, &right.0)
        });
        failed_repos.sort_by(|left, right| {
            compare_repository_locations(&left.1, &left.0, &right.1, &right.0)
        });
        outcomes.sort_by(|left, right| {
            compare_repository_locations(
                &left.path,
                &left.repository,
                &right.path,
                &right.repository,
            )
        });
        let skipped_outcomes = outcomes
            .iter()
            .filter(|outcome| is_transfer_skip(outcome.status))
            .collect::<Vec<_>>();
        let local_follow_up = outcomes
            .iter()
            .filter(|outcome| outcome.has_uncommitted && is_transfer_success(outcome.status))
            .map(|outcome| (outcome.repository.clone(), outcome.path.clone()))
            .collect::<Vec<_>>();
        let follow_up_count = local_follow_up.len() + extra_follow_up_count;
        let mut lines = Vec::new();
        let transfer_label = transfer.label();
        let totals = TransferTotals {
            transferred_repos,
            transferred_commits,
            up_to_date,
            errors,
            skipped,
            follow_up: follow_up_count,
            checked,
        };

        lines.push(format!("{BOLD_BLUE}repos {}{RESET}", transfer.command()));
        lines.push(format!("{GREEN}✓{RESET} Completed in {duration_secs:.1}s"));
        lines.push(String::new());
        lines.push(format!("{BOLD_PURPLE}▌ {transfer_label}{RESET}"));
        if transferred_details.is_empty() {
            lines.push(format!(
                "  {DIM}Nothing {} this run.{RESET}",
                transfer.verb()
            ));
        } else {
            for (repo_name, repo_path, commits) in transferred_details {
                let unit = transfer.unit(commits);
                lines.push(format!(
                    "  {GREEN}✓{RESET} {:24} {:>3} {unit}",
                    truncate_text(&repo_name, 24),
                    commits
                ));
                lines.push(format!(
                    "    {DIM}↳ path: {}{RESET}",
                    format_relative_repo_path(&repo_path)
                ));
            }
        }
        lines.push(String::new());

        let mut attention =
            transfer_failure_attention(transfer, errors, &failed_repos, &git_failures);
        attention.extend(transfer_skip_attention(transfer, &skipped_outcomes));
        attention.extend(local_follow_up.into_iter().map(|(repository, path)| {
            ProjectAttention::new(
                AttentionKind::FollowUp,
                repository,
                path,
                "uncommitted changes",
                "commit or stash the local changes",
                None,
            )
        }));
        append_project_attention_section(&mut lines, attention, show_changes);
        if !extra_follow_up_lines.is_empty() {
            lines.push(String::new());
            lines.extend(extra_follow_up_lines.iter().cloned());
            lines.push(String::new());
        }

        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }

        lines.push(String::new());
        append_transfer_summary(&mut lines, transfer, totals);

        lines.join("\n")
    }

    fn transfer_counts(&self, transfer: Transfer) -> (u64, u64) {
        match transfer {
            Transfer::Fetch => (
                self.fetched_repos.load(Ordering::Relaxed),
                self.total_refs_fetched.load(Ordering::Relaxed),
            ),
            Transfer::Push => (
                self.pushed_repos.load(Ordering::Relaxed),
                self.total_commits_pushed.load(Ordering::Relaxed),
            ),
            Transfer::Pull => (
                self.pulled_repos.load(Ordering::Relaxed),
                self.total_commits_pulled.load(Ordering::Relaxed),
            ),
        }
    }

    fn up_to_date_count(&self, transfer: Transfer) -> u64 {
        let synced = self.synced_repos.load(Ordering::Relaxed);
        let (transferred, _) = self.transfer_counts(transfer);
        synced.saturating_sub(transferred)
    }

    fn transfer_follow_up_count(&self) -> u64 {
        clone_vec(&self.operation_outcomes, "operation_outcomes")
            .iter()
            .filter(|outcome| outcome.has_uncommitted && is_transfer_success(outcome.status))
            .count() as u64
    }
}
