//! Statistics tracking for repository operations

mod format;
mod report;
mod transfer;

pub(super) use format::get_repo_changes;
pub(crate) use format::{clean_error_message, format_relative_repo_path, truncate_text};
use format::{parse_commit_count, pluralize};

use super::attention::{append_project_attention_section, AttentionKind, ProjectAttention};
use super::report::RepositoryOutcome;
use crate::git::failure::{GitFailure, GitOperationResult};
use crate::git::Status;
use crate::utils::compare_repository_locations;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

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
}
