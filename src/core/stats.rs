//! Statistics tracking for repository operations

use crate::core::config::{
    ERROR_MESSAGE_MAX_LENGTH, ERROR_MESSAGE_TRUNCATE_LENGTH, TIMEOUT_SECONDS_DISPLAY,
};
use crate::git::Status;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

const RESET: &str = "\x1b[0m";
const BOLD_BLUE: &str = "\x1b[1;38;5;75m";
const BOLD_PURPLE: &str = "\x1b[1;38;5;141m";
const GREEN: &str = "\x1b[1;38;5;114m";
const YELLOW: &str = "\x1b[1;38;5;221m";
const RED: &str = "\x1b[1;38;5;203m";
const DIM: &str = "\x1b[2m";

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
    pub skipped_repos: AtomicU64,
    pub error_repos: AtomicU64,
    pub uncommitted_count: AtomicU64,
    // Complex data behind mutex
    pub failed_repos: Mutex<Vec<(String, String, String)>>, // (repo_name, repo_path, error_message)
    pub no_upstream_repos: Mutex<Vec<(String, String)>>,    // (repo_name, repo_path)
    pub no_remote_repos: Mutex<Vec<(String, String)>>,      // (repo_name, repo_path)
    pub uncommitted_repos: Mutex<Vec<(String, String)>>,    // (repo_name, repo_path)
    pub pushed_repo_details: Mutex<Vec<(String, String, u64)>>, // (repo_name, repo_path, commits)
    pub skipped_reasons: Mutex<Vec<String>>,
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
            skipped_repos: AtomicU64::new(0),
            error_repos: AtomicU64::new(0),
            uncommitted_count: AtomicU64::new(0),
            failed_repos: Mutex::new(Vec::new()),
            no_upstream_repos: Mutex::new(Vec::new()),
            no_remote_repos: Mutex::new(Vec::new()),
            uncommitted_repos: Mutex::new(Vec::new()),
            pushed_repo_details: Mutex::new(Vec::new()),
            skipped_reasons: Mutex::new(Vec::new()),
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
                self.pushed_repos.fetch_add(1, Ordering::Relaxed);
                let commits = parse_commit_count(message).unwrap_or(0);
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
                if let Some(commits) = parse_commit_count(message) {
                    self.total_commits_pulled
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
            Status::Skip | Status::ConfigSkipped | Status::NoChanges | Status::Dirty => {
                self.skipped_repos.fetch_add(1, Ordering::Relaxed);
                self.record_skipped_reason(repo_name, skipped_reason(status, message));
            }
            Status::NoUpstream => {
                self.skipped_repos.fetch_add(1, Ordering::Relaxed);
                self.record_skipped_reason(repo_name, "no upstream");
                if let Ok(mut guard) = self.no_upstream_repos.lock() {
                    guard.push((repo_name.to_string(), repo_path.to_string()));
                } else {
                    eprintln!("Warning: Failed to record no-upstream repo: {repo_name}");
                }
            }
            Status::NoRemote => {
                self.skipped_repos.fetch_add(1, Ordering::Relaxed);
                self.record_skipped_reason(repo_name, "missing remote");
                if let Ok(mut guard) = self.no_remote_repos.lock() {
                    guard.push((repo_name.to_string(), repo_path.to_string()));
                } else {
                    eprintln!("Warning: Failed to record no-remote repo: {repo_name}");
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

    fn record_skipped_reason(&self, repo_name: &str, reason: &str) {
        if let Ok(mut guard) = self.skipped_reasons.lock() {
            guard.push(reason.to_string());
        } else {
            eprintln!("Warning: Failed to record skipped repo: {repo_name}");
        }
    }

    /// Generates a summary string of the synchronization results with enhanced formatting
    pub fn generate_summary(&self, _total_repos: usize, duration: Duration) -> String {
        self.generate_push_summary(duration)
    }

    /// Generates a push-specific completion summary.
    pub fn generate_push_summary(&self, duration: Duration) -> String {
        let duration_secs = duration.as_secs_f64();

        let synced = self.synced_repos.load(Ordering::Relaxed);
        let pushed_repos = self.pushed_repos.load(Ordering::Relaxed);
        let pushed_commits = self.total_commits_pushed.load(Ordering::Relaxed);
        let errors = self.error_repos.load(Ordering::Relaxed);

        if errors > 0 {
            format!(
                "✅ Completed in {duration_secs:.1}s • {synced} synced • {pushed_repos} pushed ({pushed_commits} commits) • {errors} failed"
            )
        } else {
            format!(
                "✅ Completed in {duration_secs:.1}s • {synced} synced • {pushed_repos} pushed ({pushed_commits} commits)"
            )
        }
    }

    /// Generates a pull/sync-specific completion summary.
    pub fn generate_pull_summary(&self, duration: Duration) -> String {
        let duration_secs = duration.as_secs_f64();

        let synced = self.synced_repos.load(Ordering::Relaxed);
        let pulled_repos = self.pulled_repos.load(Ordering::Relaxed);
        let pulled_commits = self.total_commits_pulled.load(Ordering::Relaxed);
        let errors = self.error_repos.load(Ordering::Relaxed);

        if errors > 0 {
            format!(
                "✅ Completed in {duration_secs:.1}s • {synced} synced • {pulled_repos} pulled ({pulled_commits} commits) • {errors} failed"
            )
        } else {
            format!(
                "✅ Completed in {duration_secs:.1}s • {synced} synced • {pulled_repos} pulled ({pulled_commits} commits)"
            )
        }
    }

    /// Generates a compact live push footer.
    pub fn generate_push_live_summary(&self, total_repos: usize) -> String {
        let synced = self.synced_repos.load(Ordering::Relaxed);
        let pushed_repos = self.pushed_repos.load(Ordering::Relaxed);
        let pushed_commits = self.total_commits_pushed.load(Ordering::Relaxed);
        let errors = self.error_repos.load(Ordering::Relaxed);
        let skipped = self.skipped_repos.load(Ordering::Relaxed);
        let needs_work = self.no_upstream_count() as u64
            + self.no_remote_count() as u64
            + self.uncommitted_count.load(Ordering::Relaxed);
        let processed = synced.saturating_add(errors).saturating_add(skipped);
        let remaining = (total_repos as u64).saturating_sub(processed);

        format!(
            "  {GREEN}✓{RESET} {synced} synced   {GREEN}↑{RESET} {pushed_repos} pushed / {pushed_commits} commits   {RED}!{RESET} {errors} failed   {YELLOW}!{RESET} {needs_work} needs work   {DIM}·{RESET} {skipped} skipped\n  {DIM}↳ scanning {remaining} remaining{RESET}",
        )
    }

    /// Generates the final push report without repeating the live footer details.
    pub fn generate_push_report(&self, duration: Duration, show_changes: bool) -> String {
        self.generate_push_report_with_needs_work(duration, show_changes, 0, &[])
    }

    /// Generates the final push report with additional actionable work lines.
    pub fn generate_push_report_with_needs_work(
        &self,
        duration: Duration,
        show_changes: bool,
        extra_needs_work_count: usize,
        extra_needs_work_lines: &[String],
    ) -> String {
        let duration_secs = duration.as_secs_f64();
        let synced = self.synced_repos.load(Ordering::Relaxed);
        let pushed_repos = self.pushed_repos.load(Ordering::Relaxed);
        let pushed_commits = self.total_commits_pushed.load(Ordering::Relaxed);
        let skipped = self.skipped_repos.load(Ordering::Relaxed);
        let errors = self.error_repos.load(Ordering::Relaxed);

        let pushed_details = clone_vec(&self.pushed_repo_details, "pushed_repo_details");
        let failed_repos = clone_vec(&self.failed_repos, "failed_repos");
        let no_upstream_repos = clone_vec(&self.no_upstream_repos, "no_upstream_repos");
        let no_remote_repos = clone_vec(&self.no_remote_repos, "no_remote_repos");
        let uncommitted_repos = clone_vec(&self.uncommitted_repos, "uncommitted_repos");
        let skipped_reasons = clone_vec(&self.skipped_reasons, "skipped_reasons");

        let mut issue_rows = build_issue_rows(&failed_repos, &no_upstream_repos, &no_remote_repos);
        let issue_index = issue_rows
            .iter()
            .enumerate()
            .map(|(index, row)| (row.repo.clone(), index))
            .collect::<HashMap<_, _>>();

        let mut local_only = Vec::new();
        let mut local_seen = HashSet::new();
        for (repo_name, repo_path) in &uncommitted_repos {
            if let Some(index) = issue_index.get(repo_name).copied() {
                issue_rows[index].add_reason("uncommitted changes");
            } else if local_seen.insert(repo_name.clone()) {
                local_only.push((repo_name.clone(), repo_path.clone()));
            }
        }
        let local_issue_count = local_only.len();

        let issue_count = issue_rows.len();
        let needs_work = issue_count + local_issue_count + extra_needs_work_count;
        let mut lines = Vec::new();
        let pushed_repo_label = pluralize(pushed_repos, "repo", "repos");
        let pushed_commit_label = pluralize(pushed_commits, "commit", "commits");

        lines.push(format!("{BOLD_BLUE}repos push{RESET}"));
        lines.push(format!("{GREEN}✓{RESET} Completed in {duration_secs:.1}s"));
        lines.push(String::new());
        lines.push(format!("{BOLD_PURPLE}▌ Summary{RESET}"));
        lines.push(format!("  {GREEN}✓{RESET} Synced       {synced}"));
        lines.push(format!(
            "  {GREEN}✓{RESET} Pushed       {pushed_repos} {pushed_repo_label} / {pushed_commits} {pushed_commit_label}"
        ));
        if errors > 0 {
            lines.push(format!("  {RED}!{RESET} Failed       {errors}"));
        }
        if needs_work > 0 {
            lines.push(format!("  {YELLOW}!{RESET} Needs work   {needs_work}"));
        }
        if skipped > 0 {
            lines.push(format!("  {DIM}·{RESET} Skipped      {skipped}"));
        }
        lines.push(String::new());

        lines.push(format!("{BOLD_PURPLE}▌ Pushed{RESET}"));
        if pushed_details.is_empty() {
            lines.push(format!("  {DIM}Nothing pushed this run.{RESET}"));
        } else {
            for (repo_name, _repo_path, commits) in pushed_details {
                let commit_label = if commits == 1 { "commit" } else { "commits" };
                lines.push(format!(
                    "  {GREEN}✓{RESET} {:24} {:>3} {commit_label}",
                    truncate_text(&repo_name, 24),
                    commits
                ));
            }
        }
        lines.push(String::new());

        append_failed_section(&mut lines, errors, &failed_repos);

        if skipped > 0 {
            append_skipped_section(&mut lines, skipped, &skipped_reasons);
        }

        if !issue_rows.is_empty() || !extra_needs_work_lines.is_empty() {
            lines.push(format!(
                "{BOLD_PURPLE}▌ Needs Work{RESET}{}",
                format_needs_work_detail(
                    needs_work,
                    issue_count,
                    extra_needs_work_count,
                    local_issue_count
                )
            ));
            if !issue_rows.is_empty() {
                lines.extend(format_issue_table(&issue_rows));
            }
            if !extra_needs_work_lines.is_empty() {
                if !issue_rows.is_empty() {
                    lines.push(String::new());
                }
                lines.extend(extra_needs_work_lines.iter().cloned());
            }
            lines.push(String::new());
        }

        if local_issue_count > 0 {
            append_local_changes_section(&mut lines, &local_only);
            if show_changes {
                lines.extend(format_local_changes(&local_only));
            }
            lines.push(String::new());
        }

        if needs_work > 0 || errors > 0 || skipped > 0 {
            append_next_section(
                &mut lines,
                errors,
                skipped,
                extra_needs_work_count,
                !issue_rows.is_empty() || local_issue_count > 0,
            );
        }

        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }

        lines.join("\n")
    }

    /// Generates a compact live pull/sync footer.
    pub fn generate_pull_live_summary(&self) -> String {
        format!(
            "🔽 {} Pulled / {} Commits  🟢 {} Synced  🔴 {} Failed  🟡 {} No Upstream  🟠 {} Skipped",
            self.pulled_repos.load(Ordering::Relaxed),
            self.total_commits_pulled.load(Ordering::Relaxed),
            self.synced_repos.load(Ordering::Relaxed),
            self.error_repos.load(Ordering::Relaxed),
            self.no_upstream_count(),
            self.skipped_repos.load(Ordering::Relaxed)
        )
    }

    fn no_upstream_count(&self) -> usize {
        self.no_upstream_repos.lock().map_or(0, |repos| repos.len())
    }

    fn no_remote_count(&self) -> usize {
        self.no_remote_repos.lock().map_or(0, |repos| repos.len())
    }

    /// Generates detailed warning messages for repositories needing attention
    pub fn generate_detailed_summary(&self, show_changes: bool) -> String {
        let mut lines = Vec::new();

        // Lock all vectors once at the beginning - handle lock failures gracefully
        let failed_repos = if let Ok(guard) = self.failed_repos.lock() {
            guard
        } else {
            eprintln!("Warning: Failed to acquire lock for failed_repos");
            return String::new();
        };
        let no_upstream_repos = if let Ok(guard) = self.no_upstream_repos.lock() {
            guard
        } else {
            eprintln!("Warning: Failed to acquire lock for no_upstream_repos");
            return String::new();
        };
        let no_remote_repos = if let Ok(guard) = self.no_remote_repos.lock() {
            guard
        } else {
            eprintln!("Warning: Failed to acquire lock for no_remote_repos");
            return String::new();
        };
        let uncommitted_repos = if let Ok(guard) = self.uncommitted_repos.lock() {
            guard
        } else {
            eprintln!("Warning: Failed to acquire lock for uncommitted_repos");
            return String::new();
        };

        let mut diverged_repos = Vec::new();
        let mut push_blocked_repos = Vec::new();
        let mut other_failed_repos = Vec::new();

        for repo in failed_repos.iter() {
            let error = repo.2.to_lowercase();
            if error.contains("diverged") {
                diverged_repos.push(repo);
            } else if error.contains("email privacy") {
                push_blocked_repos.push(repo);
            } else {
                other_failed_repos.push(repo);
            }
        }

        if !diverged_repos.is_empty() {
            lines.push(format!("🔴 DIVERGED ({})", diverged_repos.len()));
            for (i, (repo_name, repo_path, error)) in diverged_repos.iter().enumerate() {
                let tree_char = if i == diverged_repos.len() - 1 {
                    "└─"
                } else {
                    "├─"
                };
                lines.push(format!(
                    "   {tree_char} {repo_name:20} {repo_path:30} # {error}"
                ));
            }
            lines.push(String::new());
        }

        if !push_blocked_repos.is_empty() {
            lines.push(format!("⛔ PUSH BLOCKED ({})", push_blocked_repos.len()));
            for (i, (repo_name, repo_path, error)) in push_blocked_repos.iter().enumerate() {
                let tree_char = if i == push_blocked_repos.len() - 1 {
                    "└─"
                } else {
                    "├─"
                };
                lines.push(format!(
                    "   {tree_char} {repo_name:20} {repo_path:30} # {error}"
                ));
            }
            lines.push(String::new());
        }

        if !other_failed_repos.is_empty() {
            lines.push(format!("🔴 FAILED REPOS ({})", other_failed_repos.len()));
            for (i, (repo_name, repo_path, error)) in other_failed_repos.iter().enumerate() {
                let tree_char = if i == other_failed_repos.len() - 1 {
                    "└─"
                } else {
                    "├─"
                };
                lines.push(format!(
                    "   {tree_char} {repo_name:20} {repo_path:30} # {error}"
                ));
            }
            lines.push(String::new());
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
                lines.push(format!(
                    "   {tree_char} {repo_name:20} {repo_path:30} # repos push --auto-upstream"
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
                if show_changes {
                    // Show repo header with path
                    lines.push(format!("   {tree_char} {repo_name:20} {repo_path}"));

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
                                lines.push(format!("{prefix}  {change}"));
                            }
                        }
                    }
                } else {
                    lines.push(format!("   {tree_char} {repo_name:20} {repo_path}"));
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
                lines.push(format!("   {tree_char} {repo_name:20} {repo_path}"));
            }
        }

        // Remove trailing blank line if it exists
        if lines.last() == Some(&String::new()) {
            lines.pop();
        }

        lines.join("\n")
    }
}

#[derive(Debug)]
struct IssueRow {
    repo: String,
    path: String,
    reason: String,
    next: String,
}

impl IssueRow {
    fn add_reason(&mut self, reason: &str) {
        if self.reason.contains(reason) {
            return;
        }
        if !self.reason.is_empty() {
            self.reason.push_str(" + ");
        }
        self.reason.push_str(reason);

        if self.next == "repos push --auto-upstream" {
            self.next = "commit/clean, then auto-upstream".to_string();
        }
    }
}

fn clone_vec<T: Clone>(values: &Mutex<Vec<T>>, label: &str) -> Vec<T> {
    match values.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => {
            eprintln!("Warning: Failed to acquire lock for {label}");
            Vec::new()
        }
    }
}

fn append_failed_section(
    lines: &mut Vec<String>,
    errors: u64,
    failed_repos: &[(String, String, String)],
) {
    if errors == 0 {
        return;
    }

    lines.push(format!("{BOLD_PURPLE}▌ Failed{RESET}"));
    if failed_repos.is_empty() {
        lines.push(format!("  {RED}!{RESET} {errors} repos failed"));
        lines.push(format!("    {DIM}↳ Run `repos status --failed`{RESET}"));
    } else {
        for (repo_name, repo_path, error) in failed_repos {
            lines.push(format!(
                "  {RED}!{RESET} {:24} {}",
                truncate_text(repo_name, 24),
                compact_push_error(error)
            ));
            lines.push(format!(
                "    {DIM}↳ path: {}{RESET}",
                format_relative_repo_path(repo_path)
            ));
            lines.push(format!(
                "    {DIM}↳ next: {}{RESET}",
                next_for_push_error(error)
            ));
        }
    }
    lines.push(String::new());
}

fn append_skipped_section(lines: &mut Vec<String>, skipped: u64, skipped_reasons: &[String]) {
    lines.push(format!("{BOLD_PURPLE}▌ Skipped{RESET}"));
    lines.push(format!("  {DIM}·{RESET} {skipped} repos skipped"));
    for (reason, count) in summarize_skipped_reasons(skipped_reasons) {
        lines.push(format!("    {DIM}· {count} {reason}{RESET}"));
    }
    lines.push(format!("    {DIM}↳ Run `repos status --skipped`{RESET}"));
    lines.push(String::new());
}

fn format_needs_work_detail(
    needs_work: usize,
    issue_count: usize,
    extra_needs_work_count: usize,
    local_issue_count: usize,
) -> String {
    let mut detail_parts = Vec::new();
    if issue_count > 0 {
        detail_parts.push(format!(
            "{issue_count} {}",
            pluralize(issue_count as u64, "repo", "repos")
        ));
    }
    if extra_needs_work_count > 0 {
        detail_parts.push(format!(
            "{extra_needs_work_count} {}",
            pluralize(
                extra_needs_work_count as u64,
                "nested package group",
                "nested package groups"
            )
        ));
    }
    if local_issue_count > 0 {
        detail_parts.push(format!(
            "{local_issue_count} {}",
            pluralize(local_issue_count as u64, "dirty repo", "dirty repos")
        ));
    }

    if detail_parts.is_empty() {
        String::new()
    } else {
        format!(
            " {}",
            paint(
                DIM,
                &format!("{needs_work} total: {}", detail_parts.join(" + "))
            )
        )
    }
}

fn append_local_changes_section(lines: &mut Vec<String>, local_only: &[(String, String)]) {
    let local_names = local_only
        .iter()
        .map(|(repo_name, _)| repo_name.as_str())
        .collect::<Vec<_>>();
    let local_issue_count = local_names.len();

    lines.push(format!("{BOLD_PURPLE}▌ Local Changes{RESET}"));
    if local_issue_count == 1 {
        lines.push(format!(
            "  {YELLOW}!{RESET} 1 repo has uncommitted changes: {}",
            local_names[0]
        ));
    } else {
        lines.push(format!(
            "  {YELLOW}!{RESET} {} {} have uncommitted changes:",
            local_issue_count,
            pluralize(local_issue_count as u64, "repo", "repos")
        ));
        for repo_name in local_names {
            lines.push(format!("    {DIM}·{RESET} {repo_name}"));
        }
    }
}

fn append_next_section(
    lines: &mut Vec<String>,
    errors: u64,
    skipped: u64,
    extra_needs_work_count: usize,
    has_repo_work: bool,
) {
    let mut next_index = 1;
    lines.push(format!("{BOLD_PURPLE}▌ Next{RESET}"));
    if errors > 0 {
        push_next_step(lines, &mut next_index, "`repos status --failed`");
    }
    if skipped > 0 {
        push_next_step(lines, &mut next_index, "`repos status --skipped`");
    }
    if extra_needs_work_count > 0 {
        push_next_step(
            lines,
            &mut next_index,
            "Clean dirty nested package copies, then run `repos nested status`",
        );
        push_next_step(
            lines,
            &mut next_index,
            "Run the listed `repos nested sync ...` commands",
        );
    }
    if has_repo_work {
        push_next_step(lines, &mut next_index, "`repos status --needs-work`");
    }
}

fn push_next_step(lines: &mut Vec<String>, next_index: &mut usize, step: &str) {
    lines.push(format!("  {next_index}. {step}"));
    *next_index += 1;
}

fn build_issue_rows(
    failed_repos: &[(String, String, String)],
    no_upstream_repos: &[(String, String)],
    no_remote_repos: &[(String, String)],
) -> Vec<IssueRow> {
    let mut rows = Vec::new();
    let mut seen = HashSet::new();

    for (repo_name, repo_path, error) in failed_repos {
        if seen.insert(repo_name.clone()) {
            rows.push(IssueRow {
                repo: repo_name.clone(),
                path: repo_path.clone(),
                reason: compact_push_error(error),
                next: next_for_push_error(error),
            });
        }
    }

    for (repo_name, repo_path) in no_upstream_repos {
        if seen.insert(repo_name.clone()) {
            rows.push(IssueRow {
                repo: repo_name.clone(),
                path: repo_path.clone(),
                reason: "no upstream".to_string(),
                next: "repos push --auto-upstream".to_string(),
            });
        }
    }

    for (repo_name, repo_path) in no_remote_repos {
        if seen.insert(repo_name.clone()) {
            rows.push(IssueRow {
                repo: repo_name.clone(),
                path: repo_path.clone(),
                reason: "missing remote".to_string(),
                next: "add remote or skip".to_string(),
            });
        }
    }

    rows
}

fn compact_push_error(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("diverged") {
        return error
            .replace(" (run repos sync or resolve manually)", "")
            .replace(", ", " / ");
    }
    clean_error_message(error)
}

fn next_for_push_error(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("diverged") {
        "repos sync or resolve manually".to_string()
    } else if lower.contains("repository moved") && lower.contains("email privacy") {
        "update remote + fix git email".to_string()
    } else if lower.contains("email privacy") {
        "fix git email, then push".to_string()
    } else if lower.contains("repository moved") {
        "update remote, then push".to_string()
    } else {
        "inspect failure".to_string()
    }
}

fn format_issue_table(rows: &[IssueRow]) -> Vec<String> {
    const REPO_WIDTH: usize = 26;
    const REASON_WIDTH: usize = 36;
    const NEXT_WIDTH: usize = 34;
    const GAP: &str = "  ";

    let rule = format!(
        "  {}{GAP}{}{GAP}{}",
        "─".repeat(REPO_WIDTH),
        "─".repeat(REASON_WIDTH),
        "─".repeat(NEXT_WIDTH)
    );
    let mut lines = vec![
        paint(
            DIM,
            &format!(
                "  {:REPO_WIDTH$}{GAP}{:REASON_WIDTH$}{GAP}{:NEXT_WIDTH$}",
                "Repo", "Reason", "Next"
            ),
        ),
        paint(DIM, &rule),
    ];

    for row in rows {
        let line = format!(
            "  {:REPO_WIDTH$}{GAP}{:REASON_WIDTH$}{GAP}{:NEXT_WIDTH$}",
            truncate_text(&row.repo, REPO_WIDTH),
            truncate_text(&row.reason, REASON_WIDTH),
            truncate_text(&row.next, NEXT_WIDTH)
        );
        lines.push(paint(issue_row_color(row), &line));
        lines.push(paint(
            DIM,
            &format!("    └─ {}", format_relative_repo_path(&row.path)),
        ));
    }

    lines
}

fn issue_row_color(row: &IssueRow) -> &'static str {
    if row.reason.contains("diverged")
        || row.reason.contains("email privacy")
        || row.reason.contains("network")
    {
        RED
    } else {
        YELLOW
    }
}

fn paint(color: &str, value: &str) -> String {
    format!("{color}{value}{RESET}")
}

fn format_relative_repo_path(path: &str) -> String {
    let repo_path = Path::new(path);
    let display_path = if repo_path.is_absolute() {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| repo_path.strip_prefix(cwd).ok())
            .map_or_else(|| repo_path.to_path_buf(), Path::to_path_buf)
    } else {
        repo_path.to_path_buf()
    };

    let value = display_path.to_string_lossy();
    if value == "." || value.starts_with("./") {
        value.to_string()
    } else {
        format!("./{value}")
    }
}

fn truncate_text(value: &str, width: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= width {
        return value.to_string();
    }

    if width <= 1 {
        return "…".to_string();
    }

    let mut truncated = value.chars().take(width - 1).collect::<String>();
    truncated.push('…');
    truncated
}

fn format_local_changes(repos: &[(String, String)]) -> Vec<String> {
    let mut lines = Vec::new();
    for (repo_name, repo_path) in repos {
        if let Ok(changes) = get_repo_changes(repo_path) {
            if changes.is_empty() {
                continue;
            }
            lines.push(format!("  {repo_name}:"));
            for change in changes {
                lines.push(format!("    {change}"));
            }
        }
    }
    lines
}

fn pluralize(count: u64, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 {
        singular
    } else {
        plural
    }
}

fn skipped_reason(status: &Status, message: &str) -> &'static str {
    match status {
        Status::NoRemote => "missing remote",
        Status::NoUpstream => "no upstream",
        Status::Dirty => "uncommitted changes",
        Status::NoChanges => "nothing to do",
        Status::Skip if message.contains("detached HEAD") => "detached HEAD",
        Status::Skip => "skipped",
        Status::ConfigSkipped => "config skipped",
        _ => "skipped",
    }
}

fn summarize_skipped_reasons(skipped_reasons: &[String]) -> Vec<(String, usize)> {
    let mut counts = HashMap::<String, usize>::new();
    for reason in skipped_reasons {
        *counts.entry(reason.clone()).or_default() += 1;
    }

    let mut summarized = counts.into_iter().collect::<Vec<_>>();
    summarized.sort_by(|(left_reason, left_count), (right_reason, right_count)| {
        right_count
            .cmp(left_count)
            .then_with(|| left_reason.cmp(right_reason))
    });
    summarized
}

fn parse_commit_count(message: &str) -> Option<u64> {
    message.split_whitespace().next()?.parse::<u64>().ok()
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
            format!("timeout ({TIMEOUT_SECONDS_DISPLAY}s)")
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
        .args([
            "status",
            "--porcelain=v1",
            "--untracked-files=normal",
            "--ignore-submodules=dirty",
        ])
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
            changes.push(format!("... and {remaining} more"));
            break;
        }
        if !line.is_empty() {
            changes.push(line.to_string());
        }
    }

    Ok(changes)
}
