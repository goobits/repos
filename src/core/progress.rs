//! Progress bar management and processing context structures

use anyhow::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::config::{DEFAULT_PROGRESS_BAR_LENGTH, PROGRESS_CHARS, PROGRESS_TEMPLATE};
use super::stats::SyncStatistics;

/// Processing context that encapsulates all parameters needed for repository processing
///
/// This struct groups related parameters to reduce function argument counts and improve
/// code organization. It contains all the shared state and configuration needed for
/// concurrent repository operations.
pub struct ProcessingContext {
    /// List of discovered repositories to process
    pub repositories: Vec<(String, PathBuf)>,
    /// Maximum length of repository names for formatting alignment
    pub max_name_length: usize,
    /// Multi-progress instance for managing multiple concurrent progress bars
    pub multi_progress: MultiProgress,
    /// Styled progress bar configuration
    pub progress_style: ProgressStyle,
    /// Thread-safe statistics tracking for operation results
    pub statistics: Arc<Mutex<SyncStatistics>>,
    /// Semaphore for controlling concurrent operations
    pub semaphore: Arc<tokio::sync::Semaphore>,
    /// Maximum configured concurrency level
    pub max_concurrency: usize,
    /// Total number of repositories being processed
    pub total_repos: usize,
    /// Start time for duration calculations
    pub start_time: std::time::Instant,
}

/// Generic processing context for custom statistics types
///
/// This struct allows using custom statistics types while maintaining
/// the same parameter grouping benefits as ProcessingContext.
pub struct GenericProcessingContext<T> {
    /// List of discovered repositories to process
    pub repositories: Vec<(String, PathBuf)>,
    /// Maximum length of repository names for formatting alignment
    pub max_name_length: usize,
    /// Multi-progress instance for managing multiple concurrent progress bars
    pub multi_progress: MultiProgress,
    /// Styled progress bar configuration
    pub progress_style: ProgressStyle,
    /// Thread-safe statistics tracking for operation results
    pub statistics: Arc<Mutex<T>>,
    /// Semaphore for controlling concurrent operations
    pub semaphore: Arc<tokio::sync::Semaphore>,
    /// Total number of repositories being processed
    pub total_repos: usize,
    /// Start time for duration calculations
    pub start_time: std::time::Instant,
}

/// Creates a ProcessingContext from repositories and start time
pub fn create_processing_context(
    repositories: Vec<(String, PathBuf)>,
    start_time: std::time::Instant,
    concurrent_limit: usize,
) -> Result<ProcessingContext> {
    let total_repos = repositories.len();
    let max_name_length = repositories
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0);
    let multi_progress = MultiProgress::new();
    let progress_style = create_progress_style()?;
    let statistics = Arc::new(Mutex::new(SyncStatistics::new()));
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrent_limit));

    Ok(ProcessingContext {
        repositories,
        max_name_length,
        multi_progress,
        progress_style,
        statistics,
        semaphore,
        max_concurrency: concurrent_limit,
        total_repos,
        start_time,
    })
}

/// Creates a GenericProcessingContext with custom statistics type
pub fn create_generic_processing_context<T>(
    repositories: Vec<(String, PathBuf)>,
    start_time: std::time::Instant,
    statistics: T,
    concurrent_limit: usize,
) -> Result<GenericProcessingContext<T>> {
    let total_repos = repositories.len();
    let max_name_length = repositories
        .iter()
        .map(|(name, _)| name.len())
        .max()
        .unwrap_or(0);
    let multi_progress = MultiProgress::new();
    let progress_style = create_progress_style()?;
    let statistics = Arc::new(Mutex::new(statistics));
    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrent_limit));

    Ok(GenericProcessingContext {
        repositories,
        max_name_length,
        multi_progress,
        progress_style,
        statistics,
        semaphore,
        total_repos,
        start_time,
    })
}

/// Creates and configures a progress bar for a repository
/// Returns a configured ProgressBar with the specified repository name
pub(crate) fn create_progress_bar(
    multi: &MultiProgress,
    style: &ProgressStyle,
    repo_name: &str,
) -> ProgressBar {
    let pb = multi.add(ProgressBar::new(DEFAULT_PROGRESS_BAR_LENGTH));
    pb.set_style(style.clone());
    pb.set_prefix(format!("🟡 {}", repo_name));
    pb.set_message("syncing...");
    pb
}

/// Creates a progress bar style configuration
/// Returns a ProgressStyle configured with the application's visual styling
pub(crate) fn create_progress_style() -> Result<ProgressStyle> {
    Ok(ProgressStyle::default_bar()
        .template(PROGRESS_TEMPLATE)?
        .progress_chars(PROGRESS_CHARS))
}

/// Helper functions for semaphore and mutex access
pub async fn acquire_semaphore_permit(
    semaphore: &'_ Arc<tokio::sync::Semaphore>,
) -> tokio::sync::SemaphorePermit<'_> {
    semaphore
        .acquire()
        .await
        .expect("Failed to acquire semaphore permit")
}

pub(crate) fn acquire_stats_lock<T>(stats: &'_ Arc<Mutex<T>>) -> std::sync::MutexGuard<'_, T> {
    stats.lock().expect("Failed to acquire statistics lock")
}

/// Creates a separator progress bar for visual spacing between sections
/// Returns a finished ProgressBar that provides visual separation
pub(crate) fn create_separator_progress_bar(multi_progress: &MultiProgress) -> ProgressBar {
    let separator_pb = multi_progress.add(ProgressBar::new(0));
    separator_pb.set_style(ProgressStyle::default_bar().template(" ").expect("Failed to create separator progress bar template - this indicates an invalid template string"));
    separator_pb.finish();
    separator_pb
}

/// Creates a footer progress bar for displaying summary information
/// Returns a configured ProgressBar for showing operation summaries
pub(crate) fn create_footer_progress_bar(multi_progress: &MultiProgress) -> ProgressBar {
    let footer_pb = multi_progress.add(ProgressBar::new(0));
    let footer_style = ProgressStyle::default_bar()
        .template("{wide_msg}")
        .expect("Failed to create footer progress style - this indicates an invalid template string in the progress bar configuration");
    footer_pb.set_style(footer_style);
    footer_pb
}
