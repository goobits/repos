//! Statistics tracking for repository operations

use crate::core::config::{
    ERROR_MESSAGE_MAX_LENGTH, ERROR_MESSAGE_TRUNCATE_LENGTH, PATH_DISPLAY_WIDTH,
    TIMEOUT_SECONDS_DISPLAY,
};
use crate::git::Status;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// Statistics for tracking repository synchronization results
///
/// Uses atomic counters for lock-free reads and writes of simple counters,
/// while complex data structures (vectors) remain behind a Mutex.
#[derive(Debug)]
pub struct SyncStatistics {
    // Atomic counters for lock-free access
    pub synced_repos: AtomicU64,
    pub total_commits_pushed: AtomicU64,
    pub skipped_repos: AtomicU64,
    pub error_repos: AtomicU64,
    pub uncommitted_count: AtomicU64,
    // Complex data behind mutex
    pub failed_repos: Mutex<Vec<(String, String, String)>>, // (repo_name, repo_path, error_message)
    pub no_upstream_repos: Mutex<Vec<(String, String)>>,    // (repo_name, repo_path)
    pub no_remote_repos: Mutex<Vec<(String, String)>>,      // (repo_name, repo_path)
    pub uncommitted_repos: Mutex<Vec<(String, String)>>,    // (repo_name, repo_path)
}

impl Default for SyncStatistics {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncStatistics {
    /// Creates a new statistics tracker with all counters initialized to zero
    pub fn new() -> Self {
        Self {
            synced_repos: AtomicU64::new(0),
            total_commits_pushed: AtomicU64::new(0),
            skipped_repos: AtomicU64::new(0),
            error_repos: AtomicU64::new(0),
            uncommitted_count: AtomicU64::new(0),
            failed_repos: Mutex::new(Vec::new()),
            no_upstream_repos: Mutex::new(Vec::new()),
            no_remote_repos: Mutex::new(Vec::new()),
            uncommitted_repos: Mutex::new(Vec::new()),
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
        match status {
            Status::Pushed => {
                self.synced_repos.fetch_add(1, Ordering::Relaxed);
                // Extract number of commits from message (e.g., "3 commits pushed")
                if let Ok(commits) = message
                    .split_whitespace()
                    .next()
                    .unwrap_or("0")
                    .parse::<u64>()
                {
                    self.total_commits_pushed
                        .fetch_add(commits, Ordering::Relaxed);
                }
            }
            Status::Pulled => {
                self.synced_repos.fetch_add(1, Ordering::Relaxed);
                // Extract number of commits from message (e.g., "3 commits pulled")
                if let Ok(commits) = message
                    .split_whitespace()
                    .next()
                    .unwrap_or("0")
                    .parse::<u64>()
                {
                    self.total_commits_pushed
                        .fetch_add(commits, Ordering::Relaxed);
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
            Status::Skip | Status::ConfigSkipped | Status::NoChanges => {
                self.skipped_repos.fetch_add(1, Ordering::Relaxed);
            }
            Status::NoUpstream => {
                self.skipped_repos.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut guard) = self.no_upstream_repos.lock() {
                    guard.push((repo_name.to_string(), repo_path.to_string()));
                } else {
                    eprintln!("Warning: Failed to record no-upstream repo: {}", repo_name);
                }
            }
            Status::NoRemote => {
                self.skipped_repos.fetch_add(1, Ordering::Relaxed);
                if let Ok(mut guard) = self.no_remote_repos.lock() {
                    guard.push((repo_name.to_string(), repo_path.to_string()));
                } else {
                    eprintln!("Warning: Failed to record no-remote repo: {}", repo_name);
                }
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
                    eprintln!("Warning: Failed to record error for repo: {}", repo_name);
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
                eprintln!(
                    "Warning: Failed to record uncommitted changes for repo: {}",
                    repo_name
                );
            }
        }
    }

    /// Generates a summary string of the synchronization results with enhanced formatting
    pub fn generate_summary(&self, _total_repos: usize, duration: Duration) -> String {
        let duration_secs = duration.as_secs_f64();

        // Load atomic values
        let synced = self.synced_repos.load(Ordering::Relaxed);
        let pushed = self.total_commits_pushed.load(Ordering::Relaxed);
        let errors = self.error_repos.load(Ordering::Relaxed);

        let mut summary = String::new();

        // Main summary line
        if errors > 0 {
            summary.push_str(&format!(
                "✅ Completed in {:.1}s • {} synced • {} pushed • {} failed",
                duration_secs, synced, pushed, errors
            ));
        } else {
            summary.push_str(&format!(
                "✅ Completed in {:.1}s • {} synced • {} pushed",
                duration_secs, synced, pushed
            ));
        }

        summary
    }

    /// Generates detailed warning messages for repositories needing attention
    pub fn generate_detailed_summary(&self, show_changes: bool) -> String {
        let mut lines = Vec::new();

        // Lock all vectors once at the beginning - handle lock failures gracefully
        let failed_repos = match self.failed_repos.lock() {
            Ok(guard) => guard,
            Err(_) => {
                eprintln!("Warning: Failed to acquire lock for failed_repos");
                return String::new();
            }
        };
        let no_upstream_repos = match self.no_upstream_repos.lock() {
            Ok(guard) => guard,
            Err(_) => {
                eprintln!("Warning: Failed to acquire lock for no_upstream_repos");
                return String::new();
            }
        };
        let no_remote_repos = match self.no_remote_repos.lock() {
            Ok(guard) => guard,
            Err(_) => {
                eprintln!("Warning: Failed to acquire lock for no_remote_repos");
                return String::new();
            }
        };
        let uncommitted_repos = match self.uncommitted_repos.lock() {
            Ok(guard) => guard,
            Err(_) => {
                eprintln!("Warning: Failed to acquire lock for uncommitted_repos");
                return String::new();
            }
        };

        // Failed repos get priority
        if !failed_repos.is_empty() {
            lines.push(format!("🔴 FAILED REPOS ({})", failed_repos.len()));
            for (i, (repo_name, repo_path, error)) in failed_repos.iter().enumerate() {
                let tree_char = if i == failed_repos.len() - 1 {
                    "└─"
                } else {
                    "├─"
                };
                let short_path = crate::utils::shorten_path(repo_path, PATH_DISPLAY_WIDTH);
                lines.push(format!(
                    "   {} {:20} {:30} # {}",
                    tree_char, repo_name, short_path, error
                ));
            }
            lines.push(String::new()); // Add blank line
        }

        // No upstream repos
        if !no_upstream_repos.is_empty() {
            lines.push(format!("🟡 NEEDS UPSTREAM ({})", no_upstream_repos.len()));
            for (i, (repo_name, repo_path)) in no_upstream_repos.iter().enumerate() {
                let tree_char = if i == no_upstream_repos.len() - 1 {
                    "└─"
                } else {
                    "├─"
                };
                let short_path = crate::utils::shorten_path(repo_path, PATH_DISPLAY_WIDTH);
                lines.push(format!(
                    "   {} {:20} {:30} # git push -u origin <branch>",
                    tree_char, repo_name, short_path
                ));
            }
            lines.push(String::new()); // Add blank line
        }

        // Uncommitted changes
        if !uncommitted_repos.is_empty() {
            lines.push(format!(
                "⚠️  UNCOMMITTED CHANGES ({})",
                uncommitted_repos.len()
            ));
            for (i, (repo_name, repo_path)) in uncommitted_repos.iter().enumerate() {
                let tree_char = if i == uncommitted_repos.len() - 1 {
                    "└─"
                } else {
                    "├─"
                };
                let short_path = crate::utils::shorten_path(repo_path, PATH_DISPLAY_WIDTH);

                if show_changes {
                    // Show repo header with path
                    lines.push(format!("   {} {:20} {}", tree_char, repo_name, short_path));

                    // Get and display file changes
                    if let Ok(changes) = get_repo_changes(repo_path) {
                        if !changes.is_empty() {
                            let is_last_repo = i == uncommitted_repos.len() - 1;
                            for (file_idx, change) in changes.iter().enumerate() {
                                let is_last_file = file_idx == changes.len() - 1;
                                let prefix = if is_last_repo {
                                    if is_last_file {
                                        "      └─"
                                    } else {
                                        "      ├─"
                                    }
                                } else if is_last_file {
                                    "   │  └─"
                                } else {
                                    "   │  ├─"
                                };
                                lines.push(format!("{}  {}", prefix, change));
                            }
                        }
                    }
                } else {
                    lines.push(format!("   {} {:20} {}", tree_char, repo_name, short_path));
                }
            }
            lines.push(String::new()); // Add blank line
        }

        // No remote repos
        if !no_remote_repos.is_empty() {
            lines.push(format!("🔧 MISSING REMOTES ({})", no_remote_repos.len()));
            for (i, (repo_name, repo_path)) in no_remote_repos.iter().enumerate() {
                let tree_char = if i == no_remote_repos.len() - 1 {
                    "└─"
                } else {
                    "├─"
                };
                let short_path = crate::utils::shorten_path(repo_path, PATH_DISPLAY_WIDTH);
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
pub(crate) fn clean_error_message(error: &str) -> String {
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
        if cleaned.contains(&TIMEOUT_SECONDS_DISPLAY.to_string()) {
            format!("timeout ({}s)", TIMEOUT_SECONDS_DISPLAY)
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
        if cleaned.len() > ERROR_MESSAGE_MAX_LENGTH {
            format!("{}...", &cleaned[..ERROR_MESSAGE_TRUNCATE_LENGTH])
        } else {
            cleaned
        }
    };

    message
}

/// Gets the list of changed files in a repository using git status --porcelain
fn get_repo_changes(repo_path: &str) -> Result<Vec<String>, std::io::Error> {
    use std::path::Path;
    use std::process::Command;

    let path = Path::new(repo_path);
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(path)
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let status_output = String::from_utf8_lossy(&output.stdout);
    let mut changes = Vec::new();
    const MAX_FILES: usize = 10; // Limit to first 10 files

    for (i, line) in status_output.lines().enumerate() {
        if i >= MAX_FILES {
            let remaining = status_output.lines().count() - MAX_FILES;
            changes.push(format!("... and {} more", remaining));
            break;
        }
        if !line.is_empty() {
            changes.push(line.to_string());
        }
    }

    Ok(changes)
}
