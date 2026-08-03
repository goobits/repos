//! Statistics tracking for repository operations

use super::report::RepositoryOutcome;
use crate::core::config::{
    ERROR_MESSAGE_MAX_LENGTH, ERROR_MESSAGE_TRUNCATE_LENGTH, TIMEOUT_SECONDS_DISPLAY,
};
use crate::git::failure::GitFailure;
use crate::git::Status;
use std::collections::HashMap;
use std::path::Path;
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
    Push,
    Pull,
}

impl Transfer {
    fn command(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::Pull => "pull",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Push => "Pushed",
            Self::Pull => "Pulled",
        }
    }

    fn verb(self) -> &'static str {
        match self {
            Self::Push => "pushed",
            Self::Pull => "pulled",
        }
    }

    fn marker(self) -> &'static str {
        match self {
            Self::Push => "↑",
            Self::Pull => "↓",
        }
    }

    fn no_upstream_action(self) -> &'static str {
        match self {
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
    pub skipped_repos: AtomicU64,
    pub error_repos: AtomicU64,
    pub uncommitted_count: AtomicU64,
    // Complex data behind mutex
    pub failed_repos: Mutex<Vec<(String, String, String)>>, // (repo_name, repo_path, error_message)
    pub uncommitted_repos: Mutex<Vec<(String, String)>>,    // (repo_name, repo_path)
    pub pushed_repo_details: Mutex<Vec<(String, String, u64)>>, // (repo_name, repo_path, commits)
    pub pulled_repo_details: Mutex<Vec<(String, String, u64)>>, // (repo_name, repo_path, commits)
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
            skipped_repos: AtomicU64::new(0),
            error_repos: AtomicU64::new(0),
            uncommitted_count: AtomicU64::new(0),
            failed_repos: Mutex::new(Vec::new()),
            uncommitted_repos: Mutex::new(Vec::new()),
            pushed_repo_details: Mutex::new(Vec::new()),
            pulled_repo_details: Mutex::new(Vec::new()),
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
                let commits = parse_commit_count(message).unwrap_or(0);
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

    /// Updates statistics and retains structured Git context for actionable reports.
    pub(crate) fn update_with_failure(
        &self,
        repo_name: &str,
        repo_path: &str,
        status: &Status,
        message: &str,
        has_uncommitted: bool,
        failure: Option<&GitFailure>,
    ) {
        self.update(repo_name, repo_path, status, message, has_uncommitted);

        if let Some(failure) = failure {
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

        format!(
            "✅ Completed in {duration_secs:.1}s • {up_to_date} up to date • {transferred_repos} {verb} ({transferred_commits} commits) • {errors} failed • {skipped} skipped"
        )
    }

    /// Generates a compact live push footer.
    pub fn generate_push_live_summary(&self, total_repos: usize) -> String {
        self.generate_transfer_live_summary(Transfer::Push, total_repos)
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
        let processed = up_to_date
            .saturating_add(transferred_repos)
            .saturating_add(errors)
            .saturating_add(skipped);
        let remaining = (total_repos as u64).saturating_sub(processed);

        format!(
            "  {GREEN}✓{RESET} {up_to_date} up to date   {GREEN}{marker}{RESET} {transferred_repos} {verb} / {transferred_commits} commits   {RED}!{RESET} {errors} failed   {DIM}·{RESET} {skipped} skipped   {YELLOW}!{RESET} {follow_up} follow-up\n  {DIM}↳ scanning {remaining} remaining{RESET}",
        )
    }

    /// Generates the final push report without repeating the live footer details.
    pub fn generate_push_report(&self, duration: Duration, show_changes: bool) -> String {
        self.generate_push_report_with_follow_up(duration, show_changes, 0, &[])
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
            Transfer::Push => clone_vec(&self.pushed_repo_details, "pushed_repo_details"),
            Transfer::Pull => clone_vec(&self.pulled_repo_details, "pulled_repo_details"),
        };
        let mut failed_repos = clone_vec(&self.failed_repos, "failed_repos");
        let mut outcomes = clone_vec(&self.operation_outcomes, "operation_outcomes");
        let git_failures = clone_failure_map(&self.git_failures);

        transferred_details
            .sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        failed_repos.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        outcomes.sort_by(|left, right| {
            left.repository
                .cmp(&right.repository)
                .then_with(|| left.path.cmp(&right.path))
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
        let transferred_repo_label = pluralize(transferred_repos, "repo", "repos");
        let transferred_commit_label = pluralize(transferred_commits, "commit", "commits");

        lines.push(format!("{BOLD_BLUE}repos {}{RESET}", transfer.command()));
        lines.push(format!("{GREEN}✓{RESET} Completed in {duration_secs:.1}s"));
        lines.push(String::new());
        lines.push(format!("{BOLD_PURPLE}▌ Summary{RESET}"));
        lines.push(format!(
            "  {GREEN}✓{RESET} {transfer_label:<13}{transferred_repos} {transferred_repo_label} / {transferred_commits} {transferred_commit_label}"
        ));
        lines.push(format!(
            "  {GREEN}✓{RESET} {:<13}{up_to_date}",
            "Up to date"
        ));
        if errors > 0 {
            lines.push(format!("  {RED}!{RESET} {:<13}{errors}", "Failed"));
        }
        if skipped > 0 {
            lines.push(format!("  {DIM}·{RESET} {:<13}{skipped}", "Skipped"));
        }
        if follow_up_count > 0 {
            lines.push(format!(
                "  {YELLOW}!{RESET} {:<13}{follow_up_count}",
                "Follow-up"
            ));
        }
        lines.push(format!("  {DIM}·{RESET} {:<13}{checked}", "Checked"));
        lines.push(String::new());

        lines.push(format!("{BOLD_PURPLE}▌ {transfer_label}{RESET}"));
        if transferred_details.is_empty() {
            lines.push(format!(
                "  {DIM}Nothing {} this run.{RESET}",
                transfer.verb()
            ));
        } else {
            for (repo_name, repo_path, commits) in transferred_details {
                let commit_label = if commits == 1 { "commit" } else { "commits" };
                lines.push(format!(
                    "  {GREEN}✓{RESET} {:24} {:>3} {commit_label}",
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

        append_failed_section(&mut lines, transfer, errors, &failed_repos, &git_failures);
        append_transfer_skips(&mut lines, transfer, &skipped_outcomes);
        append_transfer_follow_up(
            &mut lines,
            &local_follow_up,
            show_changes,
            extra_follow_up_lines,
        );

        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }

        lines.join("\n")
    }

    fn transfer_counts(&self, transfer: Transfer) -> (u64, u64) {
        match transfer {
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

fn clone_vec<T: Clone>(values: &Mutex<Vec<T>>, label: &str) -> Vec<T> {
    match values.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => {
            eprintln!("Warning: Failed to acquire lock for {label}");
            Vec::new()
        }
    }
}

fn clone_failure_map(
    failures: &Mutex<HashMap<(String, String), GitFailure>>,
) -> HashMap<(String, String), GitFailure> {
    match failures.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => {
            eprintln!("Warning: Failed to acquire lock for git_failures");
            HashMap::new()
        }
    }
}

fn append_failed_section(
    lines: &mut Vec<String>,
    transfer: Transfer,
    errors: u64,
    failed_repos: &[(String, String, String)],
    git_failures: &HashMap<(String, String), GitFailure>,
) {
    if errors == 0 {
        return;
    }

    lines.push(format!("{BOLD_PURPLE}▌ Failed{RESET}"));
    if failed_repos.is_empty() {
        lines.push(format!("  {RED}!{RESET} {errors} repos failed"));
        lines.push(format!("    {DIM}↳ Run `repos doctor`{RESET}"));
    } else {
        for (repo_name, repo_path, error) in failed_repos {
            let failure = git_failures.get(&(repo_name.clone(), repo_path.clone()));
            let reason = failure
                .map(GitFailure::reason)
                .unwrap_or_else(|| compact_git_error(error));
            let display_path = format_relative_repo_path(repo_path);
            lines.push(format!(
                "  {RED}!{RESET} {:24} {}",
                truncate_text(repo_name, 24),
                reason
            ));
            lines.push(format!("    {DIM}↳ path: {}{RESET}", display_path));
            if let Some(remote) = failure.and_then(|failure| failure.remote.as_ref()) {
                lines.push(format!("    {DIM}↳ remote: {}{RESET}", remote.display()));
            }
            let next = failure.map_or_else(
                || next_for_git_error(error, transfer),
                |failure| failure.next_action(&display_path),
            );
            lines.push(format!("    {DIM}↳ next: {}{RESET}", next));
        }
    }
    lines.push(String::new());
}

fn is_transfer_success(status: Status) -> bool {
    matches!(status, Status::Synced | Status::Pushed | Status::Pulled)
}

fn is_transfer_skip(status: Status) -> bool {
    matches!(
        status,
        Status::Skip
            | Status::NoUpstream
            | Status::NoRemote
            | Status::ConfigSkipped
            | Status::NoChanges
            | Status::Dirty
    )
}

fn append_transfer_skips(
    lines: &mut Vec<String>,
    transfer: Transfer,
    outcomes: &[&RepositoryOutcome],
) {
    if outcomes.is_empty() {
        return;
    }

    lines.push(format!("{BOLD_PURPLE}▌ Skipped{RESET}"));
    for outcome in outcomes {
        let mut reason = clean_error_message(&outcome.message);
        if outcome.has_uncommitted && !reason.contains("uncommitted") {
            reason.push_str(" + uncommitted changes");
        }
        lines.push(format!(
            "  {DIM}·{RESET} {:24} {reason}",
            truncate_text(&outcome.repository, 24)
        ));
        lines.push(format!(
            "    {DIM}↳ path: {}{RESET}",
            format_relative_repo_path(&outcome.path)
        ));
        lines.push(format!(
            "    {DIM}↳ next: {}{RESET}",
            transfer_skip_next(transfer, outcome)
        ));
    }
    lines.push(String::new());
}

fn transfer_skip_next(transfer: Transfer, outcome: &RepositoryOutcome) -> &'static str {
    match outcome.status {
        Status::NoRemote => "add remote or skip",
        Status::NoUpstream => transfer.no_upstream_action(),
        Status::Dirty => "commit or stash local changes, then retry",
        Status::Skip if outcome.message.contains("detached HEAD") => "checkout a branch",
        Status::NoChanges => "no action",
        _ => "run `repos status --skipped`",
    }
}

fn append_transfer_follow_up(
    lines: &mut Vec<String>,
    local_changes: &[(String, String)],
    show_changes: bool,
    extra_lines: &[String],
) {
    if local_changes.is_empty() && extra_lines.is_empty() {
        return;
    }

    lines.push(format!(
        "{BOLD_PURPLE}▌ Follow-up{RESET} {DIM}(does not change outcome counts){RESET}"
    ));
    for (repo_name, repo_path) in local_changes {
        lines.push(format!(
            "  {YELLOW}!{RESET} {:24} uncommitted changes",
            truncate_text(repo_name, 24)
        ));
        lines.push(format!(
            "    {DIM}↳ path: {}{RESET}",
            format_relative_repo_path(repo_path)
        ));
        lines.push(format!(
            "    {DIM}↳ next: commit or stash the local changes{RESET}"
        ));
        if show_changes {
            if let Ok(changes) = get_repo_changes(repo_path) {
                for change in changes {
                    lines.push(format!("      {DIM}· {change}{RESET}"));
                }
            }
        }
    }
    if !local_changes.is_empty() && !extra_lines.is_empty() {
        lines.push(String::new());
    }
    lines.extend(extra_lines.iter().cloned());
    lines.push(String::new());
}

fn compact_git_error(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("diverged") {
        return error
            .replace(" (run repos sync or resolve manually)", "")
            .replace(", ", " / ");
    }
    clean_error_message(error)
}

fn next_for_git_error(error: &str, transfer: Transfer) -> String {
    let lower = error.to_lowercase();
    if lower.contains("diverged") {
        "repos sync or resolve manually".to_string()
    } else if lower.contains("repository moved") && lower.contains("email privacy") {
        match transfer {
            Transfer::Push => "update remote + fix git email".to_string(),
            Transfer::Pull => "update remote, then pull".to_string(),
        }
    } else if lower.contains("email privacy") {
        match transfer {
            Transfer::Push => "fix git email, then push".to_string(),
            Transfer::Pull => "inspect failure".to_string(),
        }
    } else if lower.contains("repository moved") {
        format!("update remote, then {}", transfer.command())
    } else {
        "inspect failure".to_string()
    }
}

pub(super) fn format_relative_repo_path(path: &str) -> String {
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

fn pluralize(count: u64, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 {
        singular
    } else {
        plural
    }
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
    let cleaned = redact_http_url_secrets(&cleaned);

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
    } else if lower_contains_any(
        &cleaned,
        &[
            "authentication",
            "permission denied",
            "publickey",
            "could not read username",
            "terminal prompts disabled",
        ],
    ) {
        "authentication failed".to_string()
    } else if cleaned.contains("conflict") || cleaned.contains("diverged") {
        "merge conflict".to_string()
    } else if cleaned.contains("Connection") || cleaned.contains("network") {
        "network error".to_string()
    } else {
        // Truncate long messages
        if cleaned.chars().count() > ERROR_MESSAGE_MAX_LENGTH {
            format!(
                "{}...",
                cleaned
                    .chars()
                    .take(ERROR_MESSAGE_TRUNCATE_LENGTH)
                    .collect::<String>()
            )
        } else {
            cleaned
        }
    };

    message
}

fn redact_http_url_secrets(value: &str) -> String {
    let mut redacted = String::with_capacity(value.len());
    let mut remaining = value;

    while let Some(start) = [remaining.find("https://"), remaining.find("http://")]
        .into_iter()
        .flatten()
        .min()
    {
        redacted.push_str(&remaining[..start]);
        remaining = &remaining[start..];

        let end = remaining
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '\'' | '"' | ')' | ']' | '>' | ',')
            })
            .unwrap_or(remaining.len());
        let url = &remaining[..end];
        redacted.push_str(&redact_http_url(url));
        remaining = &remaining[end..];
    }

    redacted.push_str(remaining);
    redacted
}

fn redact_http_url(url: &str) -> String {
    let Some(scheme_end) = url.find("://").map(|index| index + 3) else {
        return url.to_string();
    };
    let remainder = &url[scheme_end..];
    let authority_end = remainder.find(['/', '?', '#']).unwrap_or(remainder.len());
    let authority = &remainder[..authority_end];
    let host = authority.rsplit('@').next().unwrap_or(authority);
    let path = &remainder[authority_end..];
    let safe_path_end = path.find(['?', '#']).unwrap_or(path.len());

    format!("{}{}{}", &url[..scheme_end], host, &path[..safe_path_end])
}

fn lower_contains_any(value: &str, patterns: &[&str]) -> bool {
    let lower = value.to_lowercase();
    patterns.iter().any(|pattern| lower.contains(pattern))
}

/// Gets the list of changed files in a repository using git status --porcelain
pub(super) fn get_repo_changes(repo_path: &str) -> Result<Vec<String>, std::io::Error> {
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
