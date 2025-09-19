//! Core infrastructure for repository processing
//!
//! This module provides:
//! - Repository discovery and scanning
//! - Statistics tracking and reporting
//! - Progress bar management
//! - Terminal utilities
//! - Helper functions

use anyhow::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use walkdir::WalkDir;

use crate::git::Status;

// Constants
pub const DEFAULT_CONCURRENT_LIMIT: usize = 5; // Optimal for I/O-bound git operations
const DEFAULT_PROGRESS_BAR_LENGTH: u64 = 100;
const DEFAULT_REPO_NAME: &str = "current";
const UNKNOWN_REPO_NAME: &str = "unknown";

// UI Constants
const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";
pub const NO_REPOS_MESSAGE: &str = "No git repositories found in current directory.";
const SYNCING_MESSAGE: &str = "syncing...";
pub const CONFIG_SYNCING_MESSAGE: &str = "checking config...";
const PROGRESS_CHARS: &str = "##-";
const PROGRESS_TEMPLATE: &str = "{prefix:.bold} {wide_msg}";

// Status messages
const UNCOMMITTED_CHANGES_SUFFIX: &str = " (uncommitted changes)";

// Directories to skip during repository search
const SKIP_DIRECTORIES: &[&str] = &[
    "node_modules",
    "vendor",
    "target",
    "build",
    ".next",
    "dist",
    "__pycache__",
    ".venv",
    "venv",
];

/// Statistics for tracking repository synchronization results
#[derive(Clone, Default)]
pub struct SyncStatistics {
    pub synced_repos: u32,
    pub total_commits_pushed: u32,
    pub skipped_repos: u32,
    pub error_repos: u32,
    pub uncommitted_count: u32,
    pub failed_repos: Vec<(String, String, String)>,  // (repo_name, repo_path, error_message)
    pub no_upstream_repos: Vec<(String, String)>,     // (repo_name, repo_path)
    pub no_remote_repos: Vec<(String, String)>,       // (repo_name, repo_path)
    pub uncommitted_repos: Vec<(String, String)>,     // (repo_name, repo_path)
}

impl SyncStatistics {
    /// Creates a new statistics tracker with all counters initialized to zero
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates statistics based on the synchronization result
    pub fn update(&mut self, repo_name: &str, repo_path: &str, status: &Status, message: &str, has_uncommitted: bool) {
        match status {
            Status::Pushed => {
                self.synced_repos += 1;
                // Extract number of commits from message (e.g., "3 commits pushed")
                if let Ok(commits) = message
                    .split_whitespace()
                    .next()
                    .unwrap_or("0")
                    .parse::<u32>()
                {
                    self.total_commits_pushed += commits;
                }
            }
            Status::Synced | Status::ConfigSynced | Status::ConfigUpdated => self.synced_repos += 1,
            Status::Skip | Status::ConfigSkipped => self.skipped_repos += 1,
            Status::NoUpstream => {
                self.skipped_repos += 1;
                self.no_upstream_repos.push((repo_name.to_string(), repo_path.to_string()));
            }
            Status::NoRemote => {
                self.skipped_repos += 1;
                self.no_remote_repos.push((repo_name.to_string(), repo_path.to_string()));
            }
            Status::Error | Status::ConfigError => {
                self.error_repos += 1;
                self.failed_repos.push((repo_name.to_string(), repo_path.to_string(), message.to_string()));
            }
        }

        // Only track uncommitted changes for non-failed repos
        if has_uncommitted && !matches!(status, Status::Error | Status::ConfigError) && !self.uncommitted_repos.iter().any(|(name, _)| name == repo_name) {
            self.uncommitted_count += 1;
            self.uncommitted_repos.push((repo_name.to_string(), repo_path.to_string()));
        }
    }

    /// Generates a summary string of the synchronization results with enhanced formatting
    pub fn generate_summary(&self, _total_repos: usize, duration: Duration) -> String {
        let duration_secs = duration.as_secs_f64();

        let mut summary = String::new();

        // Main summary line
        if self.error_repos > 0 {
            summary.push_str(&format!("✅ Completed in {:.1}s • {} synced • {} pushed • {} failed",
                duration_secs, self.synced_repos, self.total_commits_pushed, self.error_repos));
        } else {
            summary.push_str(&format!("✅ Completed in {:.1}s • {} synced • {} pushed",
                duration_secs, self.synced_repos, self.total_commits_pushed));
        }

        summary
    }

    /// Generates detailed warning messages for repositories needing attention
    pub fn generate_detailed_summary(&self) -> String {
        let mut lines = Vec::new();

        // Failed repos get priority
        if !self.failed_repos.is_empty() {
            lines.push(format!("🔴 FAILED REPOS ({})", self.failed_repos.len()));
            for (i, (repo_name, repo_path, error)) in self.failed_repos.iter().enumerate() {
                let tree_char = if i == self.failed_repos.len() - 1 { "└─" } else { "├─" };
                let short_path = shorten_path(repo_path, 30);
                lines.push(format!("   {} {:20} {:30} # {}", tree_char, repo_name, short_path, error));
            }
            lines.push(String::new()); // Add blank line
        }

        // No upstream repos
        if !self.no_upstream_repos.is_empty() {
            lines.push(format!("🟡 NEEDS UPSTREAM ({})", self.no_upstream_repos.len()));
            for (i, (repo_name, repo_path)) in self.no_upstream_repos.iter().enumerate() {
                let tree_char = if i == self.no_upstream_repos.len() - 1 { "└─" } else { "├─" };
                let short_path = shorten_path(repo_path, 30);
                lines.push(format!("   {} {:20} {:30} # git push -u origin <branch>", tree_char, repo_name, short_path));
            }
            lines.push(String::new()); // Add blank line
        }

        // Uncommitted changes
        if !self.uncommitted_repos.is_empty() {
            lines.push(format!("⚠️  UNCOMMITTED CHANGES ({})", self.uncommitted_repos.len()));
            for (i, (repo_name, repo_path)) in self.uncommitted_repos.iter().enumerate() {
                let tree_char = if i == self.uncommitted_repos.len() - 1 { "└─" } else { "├─" };
                let short_path = shorten_path(repo_path, 30);
                lines.push(format!("   {} {:20} {}", tree_char, repo_name, short_path));
            }
            lines.push(String::new()); // Add blank line
        }

        // No remote repos
        if !self.no_remote_repos.is_empty() {
            lines.push(format!("🔧 MISSING REMOTES ({})", self.no_remote_repos.len()));
            for (i, (repo_name, repo_path)) in self.no_remote_repos.iter().enumerate() {
                let tree_char = if i == self.no_remote_repos.len() - 1 { "└─" } else { "├─" };
                let short_path = shorten_path(repo_path, 30);
                lines.push(format!("   {} {:20} {}", tree_char, repo_name, short_path));
            }
        }

        // Remove trailing blank line if it exists
        if lines.last() == Some(&String::new()) {
            lines.pop();
        }

        lines.join("\n")
    }
}

/// Cleans and formats error messages for display
pub fn clean_error_message(error: &str) -> String {
    // Replace newlines/tabs with spaces and collapse whitespace
    let cleaned = error
        .replace('\n', " ")
        .replace('\r', "")
        .replace('\t', " ");
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");

    // Extract key error patterns
    let message = if cleaned.contains("repository moved") {
        if cleaned.contains("email privacy") {
            "repo moved + email privacy".to_string()
        } else {
            "repo moved".to_string()
        }
    } else if cleaned.contains("email privacy") {
        "email privacy restriction".to_string()
    } else if cleaned.contains("timed out") {
        // Extract timeout duration if present
        if cleaned.contains("180") {
            "timeout (180s)".to_string()
        } else {
            "timeout".to_string()
        }
    } else if cleaned.contains("authentication") || cleaned.contains("Permission denied") {
        "authentication failed".to_string()
    } else if cleaned.contains("conflict") || cleaned.contains("diverged") {
        "merge conflict".to_string()
    } else if cleaned.contains("Connection") || cleaned.contains("network") {
        "network error".to_string()
    } else {
        // Truncate long messages
        if cleaned.len() > 40 {
            format!("{}...", &cleaned[..37])
        } else {
            cleaned
        }
    };

    message
}

/// Shortens long paths for display
pub fn shorten_path(path: &str, max_length: usize) -> String {
    if path.len() <= max_length {
        return path.to_string();
    }

    let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if components.len() <= 2 {
        // Too few components to shorten meaningfully
        return path.to_string();
    }

    // Keep last 2 components with ellipsis prefix
    let prefix = if path.starts_with("./") { "./" } else { "" };
    format!("{}.../{}/{}",
        prefix,
        components[components.len()-2],
        components[components.len()-1])
}

/// Helper function to safely acquire a mutex lock with error handling
/// Returns the lock guard or panics with a descriptive message
pub fn acquire_stats_lock(stats: &Mutex<SyncStatistics>) -> std::sync::MutexGuard<'_, SyncStatistics> {
    stats
        .lock()
        .expect("Failed to acquire lock on statistics mutex - mutex may be poisoned")
}

/// Helper function to safely acquire a semaphore permit
/// Returns the permit or panics with a descriptive message
pub async fn acquire_semaphore_permit(
    semaphore: &tokio::sync::Semaphore,
) -> tokio::sync::SemaphorePermit<'_> {
    semaphore
        .acquire()
        .await
        .expect("Failed to acquire semaphore permit for concurrent git operations")
}

/// Sets the terminal title to the specified text
pub fn set_terminal_title(title: &str) {
    // ANSI escape sequence to set terminal title
    print!("\x1b]0;{}\x07", title);
}

/// Sets the terminal title and ensures it's flushed to the terminal
pub fn set_terminal_title_and_flush(title: &str) {
    set_terminal_title(title);
    std::io::stdout().flush().unwrap();
}

/// Creates and configures a progress bar for a repository
/// Returns a configured ProgressBar with the specified repository name
pub fn create_progress_bar(
    multi: &MultiProgress,
    style: &ProgressStyle,
    repo_name: &str,
) -> ProgressBar {
    let pb = multi.add(ProgressBar::new(DEFAULT_PROGRESS_BAR_LENGTH));
    pb.set_style(style.clone());
    pb.set_prefix(format!("🟡 {}", repo_name));
    pb.set_message(SYNCING_MESSAGE);
    // Remove steady tick to prevent interference with terminal input
    // pb.enable_steady_tick(Duration::from_millis(PROGRESS_TICK_INTERVAL_MS));
    pb
}

/// Creates a progress bar style configuration
/// Returns a ProgressStyle configured with the application's visual styling
pub fn create_progress_style() -> Result<ProgressStyle> {
    Ok(ProgressStyle::default_bar()
        .template(PROGRESS_TEMPLATE)?
        .progress_chars(PROGRESS_CHARS))
}

/// Recursively searches for git repositories in the current directory
/// Returns a vector of (repository_name, path) tuples with deduplication
pub fn find_repos() -> Vec<(String, PathBuf)> {
    let mut repositories = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut name_counts = HashMap::new();

    // Walk through directory tree, skipping common build/dependency directories
    for entry in WalkDir::new(".")
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| {
            if let Some(file_name) = e.file_name().to_str() {
                !SKIP_DIRECTORIES.contains(&file_name)
            } else {
                true
            }
        })
        .flatten()
    {
        // Look for .git directories to identify repositories
        if entry.file_name() == ".git" && entry.file_type().is_dir() {
            if let Some(parent) = entry.path().parent() {
                // Skip if we've already seen this exact path
                // This treats symlinks as separate repositories per user request
                if !seen_paths.insert(parent.to_path_buf()) {
                    continue;
                }

                let base_name = if parent == Path::new(".") {
                    // If we're in the current directory, use the directory name
                    if let Ok(current_dir) = std::env::current_dir() {
                        current_dir
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(DEFAULT_REPO_NAME)
                            .to_string()
                    } else {
                        DEFAULT_REPO_NAME.to_string()
                    }
                } else {
                    parent
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(UNKNOWN_REPO_NAME)
                        .to_string()
                };

                // Handle duplicate names by adding a suffix
                let count = name_counts.entry(base_name.clone()).or_insert(0);
                *count += 1;
                let repo_name = if *count > 1 {
                    format!("{}-{}", base_name, count)
                } else {
                    base_name
                };

                repositories.push((repo_name, parent.to_path_buf()));
            }
        }
    }

    // Sort repositories alphabetically by name (case-insensitive)
    repositories.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

    repositories
}

/// Common initialization for both sync and user commands
pub fn init_command(scanning_msg: &str) -> (std::time::Instant, Vec<(String, PathBuf)>) {
    println!();
    print!("{}", scanning_msg);
    std::io::stdout().flush().unwrap();

    let start_time = std::time::Instant::now();
    let repos = find_repos();

    (start_time, repos)
}

/// Common setup for repository processing
pub fn setup_processing(
    repos: &[(String, PathBuf)],
) -> Result<(usize, MultiProgress, ProgressStyle, Arc<Mutex<SyncStatistics>>, Arc<tokio::sync::Semaphore>)> {
    let max_name_length = repos.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
    let multi_progress = MultiProgress::new();
    let progress_style = create_progress_style()?;
    let statistics = Arc::new(Mutex::new(SyncStatistics::new()));
    let semaphore = Arc::new(tokio::sync::Semaphore::new(DEFAULT_CONCURRENT_LIMIT));

    Ok((max_name_length, multi_progress, progress_style, statistics, semaphore))
}

