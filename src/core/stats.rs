//! Statistics tracking for repository operations

use crate::git::Status;
use crate::core::config::{PATH_DISPLAY_WIDTH, ERROR_MESSAGE_MAX_LENGTH, ERROR_MESSAGE_TRUNCATE_LENGTH, TIMEOUT_SECONDS_DISPLAY};
use std::time::Duration;

/// Statistics for tracking repository synchronization results
#[derive(Clone, Default, Debug)]
pub struct SyncStatistics {
    pub synced_repos: u32,
    pub total_commits_pushed: u32,
    pub skipped_repos: u32,
    pub error_repos: u32,
    pub uncommitted_count: u32,
    pub failed_repos: Vec<(String, String, String)>, // (repo_name, repo_path, error_message)
    pub no_upstream_repos: Vec<(String, String)>,    // (repo_name, repo_path)
    pub no_remote_repos: Vec<(String, String)>,      // (repo_name, repo_path)
    pub uncommitted_repos: Vec<(String, String)>,    // (repo_name, repo_path)
}

impl SyncStatistics {
    /// Creates a new statistics tracker with all counters initialized to zero
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates statistics based on the synchronization result
    pub fn update(
        &mut self,
        repo_name: &str,
        repo_path: &str,
        status: &Status,
        message: &str,
        has_uncommitted: bool,
    ) {
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
            Status::Synced | Status::ConfigSynced | Status::ConfigUpdated | Status::Staged | Status::Unstaged => self.synced_repos += 1,
            Status::Skip | Status::ConfigSkipped | Status::NoChanges => self.skipped_repos += 1,
            Status::NoUpstream => {
                self.skipped_repos += 1;
                self.no_upstream_repos
                    .push((repo_name.to_string(), repo_path.to_string()));
            }
            Status::NoRemote => {
                self.skipped_repos += 1;
                self.no_remote_repos
                    .push((repo_name.to_string(), repo_path.to_string()));
            }
            Status::Error | Status::ConfigError | Status::StagingError => {
                self.error_repos += 1;
                self.failed_repos.push((
                    repo_name.to_string(),
                    repo_path.to_string(),
                    message.to_string(),
                ));
            }
        }

        // Only track uncommitted changes for non-failed repos
        if has_uncommitted
            && !matches!(status, Status::Error | Status::ConfigError | Status::StagingError)
            && !self
                .uncommitted_repos
                .iter()
                .any(|(name, _)| name == repo_name)
        {
            self.uncommitted_count += 1;
            self.uncommitted_repos
                .push((repo_name.to_string(), repo_path.to_string()));
        }
    }

    /// Generates a summary string of the synchronization results with enhanced formatting
    pub fn generate_summary(&self, _total_repos: usize, duration: Duration) -> String {
        let duration_secs = duration.as_secs_f64();

        let mut summary = String::new();

        // Main summary line
        if self.error_repos > 0 {
            summary.push_str(&format!(
                "✅ Completed in {:.1}s • {} synced • {} pushed • {} failed",
                duration_secs, self.synced_repos, self.total_commits_pushed, self.error_repos
            ));
        } else {
            summary.push_str(&format!(
                "✅ Completed in {:.1}s • {} synced • {} pushed",
                duration_secs, self.synced_repos, self.total_commits_pushed
            ));
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
                let tree_char = if i == self.failed_repos.len() - 1 {
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
        if !self.no_upstream_repos.is_empty() {
            lines.push(format!(
                "🟡 NEEDS UPSTREAM ({})",
                self.no_upstream_repos.len()
            ));
            for (i, (repo_name, repo_path)) in self.no_upstream_repos.iter().enumerate() {
                let tree_char = if i == self.no_upstream_repos.len() - 1 {
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
        if !self.uncommitted_repos.is_empty() {
            lines.push(format!(
                "⚠️  UNCOMMITTED CHANGES ({})",
                self.uncommitted_repos.len()
            ));
            for (i, (repo_name, repo_path)) in self.uncommitted_repos.iter().enumerate() {
                let tree_char = if i == self.uncommitted_repos.len() - 1 {
                    "└─"
                } else {
                    "├─"
                };
                let short_path = crate::utils::shorten_path(repo_path, PATH_DISPLAY_WIDTH);
                lines.push(format!("   {} {:20} {}", tree_char, repo_name, short_path));
            }
            lines.push(String::new()); // Add blank line
        }

        // No remote repos
        if !self.no_remote_repos.is_empty() {
            lines.push(format!(
                "🔧 MISSING REMOTES ({})",
                self.no_remote_repos.len()
            ));
            for (i, (repo_name, repo_path)) in self.no_remote_repos.iter().enumerate() {
                let tree_char = if i == self.no_remote_repos.len() - 1 {
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
