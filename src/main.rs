//! sync-repos: A tool for synchronizing multiple git repositories
//! This tool scans for git repositories and pushes any unpushed commits to their upstream remotes.

use anyhow::Result;
use clap::{Arg, Command as ClapCommand};
use futures::stream::{FuturesUnordered, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::io::Write;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::Command;
use walkdir::WalkDir;

// Constants for magic numbers and strings  
const DEFAULT_CONCURRENT_LIMIT: usize = 5; // Optimal for I/O-bound git operations
const DEFAULT_PROGRESS_BAR_LENGTH: u64 = 100;
const DEFAULT_REPO_NAME: &str = "current";
const UNKNOWN_REPO_NAME: &str = "unknown";
const DETACHED_HEAD_BRANCH: &str = "HEAD";

// Timeout constants
const GIT_OPERATION_TIMEOUT_SECS: u64 = 180; // 3 minutes per repository

// UI Constants
const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";
const NO_REPOS_MESSAGE: &str = "No git repositories found in current directory.";
const SYNCING_MESSAGE: &str = "syncing...";
const PROGRESS_CHARS: &str = "##-";
const PROGRESS_TEMPLATE: &str = "{prefix:.bold} {wide_msg}";

// Status messages
const STATUS_SYNCED: &str = "up to date";
const STATUS_NO_REMOTE: &str = "no remote";
const STATUS_DETACHED_HEAD: &str = "detached HEAD";
const STATUS_NO_UPSTREAM: &str = "no tracking";
const UNCOMMITTED_CHANGES_SUFFIX: &str = " (uncommitted changes)";

// Git command arguments
const GIT_DIFF_INDEX_ARGS: &[&str] = &["diff-index", "--quiet", "HEAD", "--"];
const GIT_REMOTE_ARGS: &[&str] = &["remote"];
const GIT_REV_PARSE_HEAD_ARGS: &[&str] = &["rev-parse", "--abbrev-ref", "HEAD"];
const GIT_FETCH_ARGS: &[&str] = &["fetch", "--quiet"];
const GIT_PUSH_ARGS: &[&str] = &["push"];

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
struct SyncStatistics {
    synced_repos: u32,
    total_commits_pushed: u32,
    skipped_repos: u32,
    error_repos: u32,
    uncommitted_count: u32,
    failed_repos: Vec<(String, String, String)>,  // (repo_name, repo_path, error_message)
    no_upstream_repos: Vec<(String, String)>,     // (repo_name, repo_path)
    no_remote_repos: Vec<(String, String)>,       // (repo_name, repo_path)
    uncommitted_repos: Vec<(String, String)>,     // (repo_name, repo_path)
}

impl SyncStatistics {
    /// Creates a new statistics tracker with all counters initialized to zero
    fn new() -> Self {
        Self::default()
    }

    /// Updates statistics based on the synchronization result
    fn update(&mut self, repo_name: &str, repo_path: &str, status: &Status, message: &str, has_uncommitted: bool) {
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
            Status::Synced => self.synced_repos += 1,
            Status::Skip => self.skipped_repos += 1,
            Status::NoUpstream => {
                self.skipped_repos += 1;
                self.no_upstream_repos.push((repo_name.to_string(), repo_path.to_string()));
            }
            Status::NoRemote => {
                self.skipped_repos += 1;
                self.no_remote_repos.push((repo_name.to_string(), repo_path.to_string()));
            }
            Status::Error => {
                self.error_repos += 1;
                self.failed_repos.push((repo_name.to_string(), repo_path.to_string(), message.to_string()));
            }
        }
        
        // Only track uncommitted changes for non-failed repos
        if has_uncommitted && !matches!(status, Status::Error) && !self.uncommitted_repos.iter().any(|(name, _)| name == repo_name) {
            self.uncommitted_count += 1;
            self.uncommitted_repos.push((repo_name.to_string(), repo_path.to_string()));
        }
    }

    /// Generates a summary string of the synchronization results with enhanced formatting
    fn generate_summary(&self, _total_repos: usize, duration: Duration) -> String {
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
    fn generate_detailed_summary(&self) -> String {
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

/// Represents the synchronization status of a git repository
#[derive(Clone)]
enum Status {
    /// Repository is already up to date with remote
    Synced,
    /// Repository had commits that were successfully pushed
    Pushed,
    /// Repository was skipped (no remote, detached HEAD, etc.)
    Skip,
    /// Repository has no upstream tracking branch
    NoUpstream,
    /// Repository has no remote configured
    NoRemote,
    /// An error occurred during synchronization
    Error,
}

impl Status {
    /// Returns the emoji symbol for this status
    fn symbol(&self) -> &str {
        match self {
            Status::Synced | Status::Pushed => "🟢",
            Status::Skip | Status::NoRemote => "🟠",
            Status::NoUpstream => "🟡",
            Status::Error => "🔴",
        }
    }

    /// Returns the text representation of this status
    fn text(&self) -> &str {
        match self {
            Status::Synced => "synced",
            Status::Pushed => "pushed",
            Status::Skip => "skip",
            Status::NoUpstream => "no-upstream",
            Status::NoRemote => "skip",
            Status::Error => "failed",
        }
    }
}

/// Cleans and formats error messages for display
fn clean_error_message(error: &str) -> String {
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
fn shorten_path(path: &str, max_length: usize) -> String {
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

/// Runs a git command in the specified directory with a timeout
/// Returns (success, stdout, stderr)
async fn run_git(path: &Path, args: &[&str]) -> Result<(bool, String, String)> {
    let timeout_duration = Duration::from_secs(GIT_OPERATION_TIMEOUT_SECS);
    
    let result = tokio::time::timeout(
        timeout_duration,
        Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
    ).await;

    match result {
        Ok(Ok(output)) => Ok((
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        )),
        Ok(Err(e)) => Err(e.into()),
        Err(_) => Err(anyhow::anyhow!(
            "Git operation timed out after {} seconds", 
            GIT_OPERATION_TIMEOUT_SECS
        )),
    }
}

/// Helper function to safely acquire a mutex lock with error handling
/// Returns the lock guard or panics with a descriptive message
fn acquire_stats_lock(stats: &Mutex<SyncStatistics>) -> std::sync::MutexGuard<'_, SyncStatistics> {
    stats
        .lock()
        .expect("Failed to acquire lock on statistics mutex - mutex may be poisoned")
}

/// Helper function to safely acquire a semaphore permit
/// Returns the permit or panics with a descriptive message  
async fn acquire_semaphore_permit(
    semaphore: &tokio::sync::Semaphore,
) -> tokio::sync::SemaphorePermit<'_> {
    semaphore
        .acquire()
        .await
        .expect("Failed to acquire semaphore permit for concurrent git operations")
}

/// Sets the terminal title to the specified text
fn set_terminal_title(title: &str) {
    // ANSI escape sequence to set terminal title
    print!("\x1b]0;{}\x07", title);
}

/// Sets the terminal title and ensures it's flushed to the terminal
fn set_terminal_title_and_flush(title: &str) {
    set_terminal_title(title);
    std::io::stdout().flush().unwrap();
}

/// Checks a git repository and attempts to push any unpushed commits
/// Returns (status, message, has_uncommitted_changes)
async fn check_repo(path: &Path, force_push: bool) -> (Status, String, bool) {
    // Check uncommitted changes
    let has_uncommitted_changes = !run_git(path, GIT_DIFF_INDEX_ARGS)
        .await
        .map(|(success, _, _)| success)
        .unwrap_or(false);

    // Check if repository has any remotes configured
    if let Ok((true, remotes, _)) = run_git(path, GIT_REMOTE_ARGS).await {
        if remotes.is_empty() {
            return (
                Status::NoRemote,
                STATUS_NO_REMOTE.to_string(),
                has_uncommitted_changes,
            );
        }
    } else {
        return (
            Status::NoRemote,
            STATUS_NO_REMOTE.to_string(),
            has_uncommitted_changes,
        );
    }

    // Get current branch
    let current_branch = match run_git(path, GIT_REV_PARSE_HEAD_ARGS).await {
        Ok((true, branch_name, _)) if branch_name != DETACHED_HEAD_BRANCH => branch_name,
        _ => {
            return (
                Status::Skip,
                STATUS_DETACHED_HEAD.to_string(),
                has_uncommitted_changes,
            )
        }
    };

    // Check if current branch has an upstream configured
    if !run_git(
        path,
        &[
            "rev-parse",
            "--abbrev-ref",
            &format!("{}@{{upstream}}", current_branch),
        ],
    )
    .await
    .map(|(success, _, _)| success)
    .unwrap_or(false)
    {
        if force_push {
            // Force push: set up upstream and push
            match run_git(path, &["push", "-u", "origin", &current_branch]).await {
                Ok((true, _, _)) => {
                    return (
                        Status::Pushed,
                        format!("{} (upstream set)", current_branch),
                        has_uncommitted_changes,
                    );
                }
                Ok((false, _, err)) => {
                    return (
                        Status::Error,
                        clean_error_message(&format!("upstream setup failed: {}", err)),
                        has_uncommitted_changes,
                    );
                }
                Err(e) => {
                    return (
                        Status::Error,
                        clean_error_message(&format!("upstream setup error: {}", e)),
                        has_uncommitted_changes,
                    );
                }
            }
        } else {
            return (
                Status::NoUpstream,
                format!("{} ({})", current_branch, STATUS_NO_UPSTREAM),
                has_uncommitted_changes,
            );
        }
    }

    // Fetch latest changes from remote
    if let Ok((false, _, err)) = run_git(path, GIT_FETCH_ARGS).await {
        return (
            Status::Error,
            clean_error_message(&format!("fetch failed: {}", err)),
            has_uncommitted_changes,
        );
    }

    // Count commits that are ahead of upstream
    let unpushed_commits = run_git(
        path,
        &[
            "rev-list",
            "--count",
            &format!("{}@{{upstream}}..HEAD", current_branch),
        ],
    )
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

    if unpushed_commits > 0 {
        // Attempt to push the unpushed commits
        match run_git(path, GIT_PUSH_ARGS).await {
            Ok((true, _, _)) => (
                Status::Pushed,
                format!("{} commits pushed", unpushed_commits),
                has_uncommitted_changes,
            ),
            Ok((false, _, err)) => {
                (
                    Status::Error,
                    clean_error_message(&err),
                    has_uncommitted_changes,
                )
            }
            Err(e) => (
                Status::Error,
                clean_error_message(&format!("push error: {}", e)),
                has_uncommitted_changes,
            ),
        }
    } else {
        (
            Status::Synced,
            STATUS_SYNCED.to_string(),
            has_uncommitted_changes,
        )
    }
}

/// Creates and configures a progress bar for a repository
/// Returns a configured ProgressBar with the specified repository name
fn create_progress_bar(
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
fn create_progress_style() -> Result<ProgressStyle> {
    Ok(ProgressStyle::default_bar()
        .template(PROGRESS_TEMPLATE)?
        .progress_chars(PROGRESS_CHARS))
}

/// Processes all repositories concurrently and updates statistics
/// Returns when all repository operations are complete
async fn process_repositories(
    repositories: Vec<(String, PathBuf)>,
    max_name_length: usize,
    multi_progress: MultiProgress,
    progress_style: ProgressStyle,
    statistics: Arc<Mutex<SyncStatistics>>,
    semaphore: Arc<tokio::sync::Semaphore>,
    total_repos: usize,
    start_time: std::time::Instant,
    force_push: bool,
) {
    let mut futures = FuturesUnordered::new();

    // First, create all repository progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in &repositories {
        let progress_bar = create_progress_bar(&multi_progress, &progress_style, repo_name);
        repo_progress_bars.push(progress_bar);
    }

    // Add a blank line before the footer (using a space to make it visible)
    let separator_pb = multi_progress.add(ProgressBar::new(0));
    separator_pb.set_style(ProgressStyle::default_bar().template(" ").unwrap());
    separator_pb.finish();

    // Finally, create the footer progress bar at the bottom
    let footer_pb = multi_progress.add(ProgressBar::new(0));
    let footer_style = ProgressStyle::default_bar()
        .template("{wide_msg}")
        .expect("Failed to create footer progress style");
    footer_pb.set_style(footer_style);
    
    // Initial footer display
    let initial_stats = SyncStatistics::new();
    let initial_summary = initial_stats.generate_summary(total_repos, start_time.elapsed());
    footer_pb.set_message(initial_summary);

    // Add another blank line after the footer (using a space to make it visible)
    let separator_pb2 = multi_progress.add(ProgressBar::new(0));
    separator_pb2.set_style(ProgressStyle::default_bar().template(" ").unwrap());
    separator_pb2.finish();

    for ((repo_name, repo_path), progress_bar) in repositories.into_iter().zip(repo_progress_bars) {
        let stats_clone = Arc::clone(&statistics);
        let semaphore_clone = Arc::clone(&semaphore);
        let footer_clone = footer_pb.clone();

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore_clone).await;

            let (status, message, has_uncommitted_changes) = check_repo(&repo_path, force_push).await;

            let display_message =
                if has_uncommitted_changes && matches!(status, Status::Synced | Status::Pushed) {
                    format!("{}{}", message, UNCOMMITTED_CHANGES_SUFFIX)
                } else {
                    message.clone()
                };

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                repo_name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<10}   {}", status.text(), display_message));
            progress_bar.finish();

            // Update statistics based on operation result
            let mut stats_guard = acquire_stats_lock(&stats_clone);
            let repo_path_str = repo_path.to_string_lossy();
            stats_guard.update(&repo_name, &repo_path_str, &status, &message, has_uncommitted_changes);
            
            // Update the footer summary after each repository completes
            let current_stats = stats_guard.clone();
            drop(stats_guard);
            
            let duration = start_time.elapsed();
            let summary = current_stats.generate_summary(total_repos, duration);
            footer_clone.set_message(summary);
        };

        futures.push(future);
    }

    // Wait for all repository operations to complete
    while futures.next().await.is_some() {}
    
    // Finish the footer progress bar
    footer_pb.finish();
    
    // Print the final detailed summary if there are any issues to report
    let final_stats = acquire_stats_lock(&statistics);
    let detailed_summary = final_stats.generate_detailed_summary();
    if !detailed_summary.is_empty() {
        println!("\n{}", "━".repeat(70));
        println!("{}", detailed_summary);
        println!("{}", "━".repeat(70));
    }
    
    // Add final spacing
    println!();
}

/// Recursively searches for git repositories in the current directory
/// Returns a vector of (repository_name, path) tuples with deduplication
fn find_repos() -> Vec<(String, PathBuf)> {
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
                // Get canonical path to handle symlinks and duplicates
                let canonical_path = parent.canonicalize().unwrap_or_else(|_| parent.to_path_buf());
                
                // Skip if we've already seen this path
                if !seen_paths.insert(canonical_path.clone()) {
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

#[tokio::main]
async fn main() -> Result<()> {
    let matches = ClapCommand::new("sync-repos")
        .version("1.0")
        .about("A tool for synchronizing multiple git repositories")
        .arg(
            Arg::new("force")
                .long("force")
                .help("Automatically push branches with no upstream tracking")
                .action(clap::ArgAction::SetTrue),
        )
        .get_matches();

    let force_push = matches.get_flag("force");
    
    // Set terminal title to indicate sync-repos is running
    set_terminal_title("🚀 sync-repos");
    
    println!();
    print!("{}", SCANNING_MESSAGE);
    std::io::stdout().flush().unwrap();

    let start_time = std::time::Instant::now();
    let repos = find_repos();
    if repos.is_empty() {
        println!("\r{}", NO_REPOS_MESSAGE);
        // Set terminal title to green checkbox to indicate completion
        set_terminal_title_and_flush("✅ sync-repos");
        return Ok(());
    }

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 { "repository" } else { "repositories" };
    print!("\r🚀 Syncing {} {}                    \n", total_repos, repo_word);
    println!();

    // Setup for concurrent processing
    let max_name_length = repos.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
    let multi_progress = MultiProgress::new();
    let progress_style = match create_progress_style() {
        Ok(style) => style,
        Err(e) => {
            // If progress style creation fails, set completion title and return error
            set_terminal_title_and_flush("✅ sync-repos");
            return Err(e);
        }
    };
    let statistics = Arc::new(Mutex::new(SyncStatistics::new()));
    let semaphore = Arc::new(tokio::sync::Semaphore::new(DEFAULT_CONCURRENT_LIMIT));

    // Process all repositories concurrently
    process_repositories(
        repos,
        max_name_length,
        multi_progress,
        progress_style,
        statistics.clone(),
        semaphore,
        total_repos,
        start_time,
        force_push,
    )
    .await;

    // Set terminal title to green checkbox to indicate completion
    set_terminal_title_and_flush("✅ sync-repos");

    Ok(())
}
