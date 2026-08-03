//! Repository sync, push, and pull command implementation
//!
//! This module handles discovering repositories, pulling safe remote changes,
//! and pushing unpushed commits to upstream remotes.

use anyhow::Result;

use crate::core::{
    create_processing_context, generate_sync_report, init_command, set_terminal_title,
    set_terminal_title_and_flush, NO_REPOS_MESSAGE,
};
use crate::git::Status;

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";
const RESET: &str = "\x1b[0m";
const GREEN: &str = "\x1b[1;38;5;114m";
const YELLOW: &str = "\x1b[1;38;5;221m";
const RED: &str = "\x1b[1;38;5;203m";
const DIM: &str = "\x1b[2m";

fn format_live_repo_status(repo_name: &str, status: Status) -> String {
    let (color, marker, label) = match status {
        Status::Fetched | Status::Pushed | Status::Pulled | Status::Synced => {
            (GREEN, "✓", status.text())
        }
        Status::NoUpstream | Status::NoRemote | Status::Dirty => (YELLOW, "!", "needs work"),
        Status::Skip | Status::NoChanges | Status::ConfigSkipped => (DIM, "·", "skipped"),
        Status::Error
        | Status::ConfigError
        | Status::StagingError
        | Status::CommitError
        | Status::PullError => (RED, "!", "failed"),
        _ => (DIM, "·", status.text()),
    };

    format!("{repo_name}  {color}{marker}{RESET} {label}")
}

type ProgressBars = (
    Vec<Option<indicatif::ProgressBar>>,
    indicatif::ProgressBar,
    Option<indicatif::ProgressBar>,
);

struct TransferRun {
    statistics: std::sync::Arc<std::sync::Mutex<crate::core::SyncStatistics>>,
    error_count: u64,
}

#[derive(Clone, Copy)]
enum FleetTransfer {
    Fetch,
    Push,
    Pull,
}

impl FleetTransfer {
    fn command(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::Push => "push",
            Self::Pull => "pull",
        }
    }

    fn activity(self) -> &'static str {
        match self {
            Self::Fetch => "Fetching",
            Self::Push => "Pushing",
            Self::Pull => "Pulling",
        }
    }

    fn running_title(self) -> &'static str {
        match self {
            Self::Fetch => "🔄 repos fetch",
            Self::Push => "🚀 repos push",
            Self::Pull => "🔽 repos pull",
        }
    }

    fn completed_title(self) -> &'static str {
        match self {
            Self::Fetch => "✅ repos fetch",
            Self::Push => "✅ repos push",
            Self::Pull => "✅ repos pull",
        }
    }
}

impl TransferRun {
    fn ensure_success(&self, operation: &str) -> Result<()> {
        if self.error_count > 0 {
            anyhow::bail!("{} repositories failed to {operation}", self.error_count);
        }
        Ok(())
    }
}

async fn prepare_transfer_context(
    operation: FleetTransfer,
    verbose: bool,
    jobs: Option<usize>,
    sequential: bool,
    qualifier: &str,
) -> Result<Option<crate::core::ProcessingContext>> {
    use crate::core::config::get_git_concurrency;

    set_terminal_title(operation.running_title());
    let (start_time, repos) = init_command(SCANNING_MESSAGE).await;
    println!();

    if repos.is_empty() {
        println!("\r{NO_REPOS_MESSAGE}");
        set_terminal_title_and_flush(operation.completed_title());
        return Ok(None);
    }

    let concurrent_limit = get_git_concurrency(jobs, sequential);
    if verbose {
        let repo_word = if repos.len() == 1 {
            "repository"
        } else {
            "repositories"
        };
        println!(
            "{} {} {repo_word}{qualifier} ({concurrent_limit} concurrent)\n",
            operation.activity(),
            repos.len()
        );
    }

    match create_processing_context(std::sync::Arc::new(repos), start_time, concurrent_limit) {
        Ok(context) => Ok(Some(context)),
        Err(error) => {
            set_terminal_title_and_flush(operation.completed_title());
            Err(error)
        }
    }
}

fn create_sync_progress(
    context: &crate::core::ProcessingContext,
    verbose: bool,
    concise_message: &str,
    footer_message: String,
) -> ProgressBars {
    use indicatif::{ProgressBar, ProgressStyle};

    let (repo_bars, single_bar) = if verbose {
        let bars = context
            .repositories
            .iter()
            .map(|(repo_name, _)| {
                let bar = crate::core::create_progress_bar(
                    &context.multi_progress,
                    &context.progress_style,
                    repo_name,
                );
                bar.set_message("processing...");
                Some(bar)
            })
            .collect();
        (bars, None)
    } else {
        let bar = context
            .multi_progress
            .add(ProgressBar::new(context.total_repos as u64));
        if let Ok(style) = ProgressStyle::default_bar().template("[{pos}/{len}] {msg}") {
            bar.set_style(style);
        }
        bar.set_message(concise_message.to_string());
        (vec![None; context.repositories.len()], Some(bar))
    };

    let _separator = crate::core::create_separator_progress_bar(&context.multi_progress);
    let footer = crate::core::create_footer_progress_bar(&context.multi_progress);
    footer.set_message(footer_message);
    let _bottom_separator = crate::core::create_separator_progress_bar(&context.multi_progress);

    (repo_bars, footer, single_bar)
}

fn record_semaphore_error(
    operation: &str,
    repo_name: &str,
    repo_path: &std::path::Path,
    error: &tokio::sync::AcquireError,
    statistics: &std::sync::Arc<std::sync::Mutex<crate::core::SyncStatistics>>,
    repo_bar: Option<&indicatif::ProgressBar>,
    single_bar: Option<&indicatif::ProgressBar>,
) {
    eprintln!("Error: Failed to acquire {operation} permit for {repo_name}: {error}");
    let message = format!("semaphore error: {error}");
    crate::core::acquire_stats_lock(statistics).update(
        repo_name,
        &repo_path.to_string_lossy(),
        &Status::Error,
        &message,
        false,
    );

    if let Some(bar) = repo_bar {
        bar.finish_with_message(format!("🔴 {repo_name}  semaphore error"));
    } else if let Some(bar) = single_bar {
        bar.set_message(format_live_repo_status(repo_name, Status::Error));
        bar.inc(1);
    }
}

fn spawn_slow_repo_watchdog(
    progress_bar: Option<&indicatif::ProgressBar>,
    repo_name: &str,
    delay: std::time::Duration,
) -> Option<tokio::task::JoinHandle<()>> {
    progress_bar.map(|progress_bar| {
        let progress_bar = progress_bar.clone();
        let repo_name = repo_name.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            progress_bar.set_message(format!("{repo_name} · still running..."));
        })
    })
}

async fn stop_slow_repo_watchdog(watchdog: &mut Option<tokio::task::JoinHandle<()>>) {
    if let Some(watchdog) = watchdog.take() {
        watchdog.abort();
        let _ = watchdog.await;
    }
}

/// Handles the two-way repository sync command.
///
/// Sync is the daily workflow: pull safe remote changes with rebase, then push
/// local commits against one discovered repository set and report both phases
/// together. The directional commands remain available for explicit control.
pub async fn handle_sync_command(
    auto_upstream: bool,
    verbose: bool,
    show_changes: bool,
    no_drift_check: bool,
    jobs: Option<usize>,
    sequential: bool,
) -> Result<()> {
    use crate::core::config::get_git_concurrency;

    set_terminal_title("🔄 repos sync");
    let (start_time, repos) = init_command(SCANNING_MESSAGE).await;
    println!();

    if repos.is_empty() {
        println!("\r{NO_REPOS_MESSAGE}");
        set_terminal_title_and_flush("✅ repos sync");
        return Ok(());
    }

    let concurrent_limit = get_git_concurrency(jobs, sequential);
    let repositories = std::sync::Arc::new(repos);
    if verbose {
        let repo_word = if repositories.len() == 1 {
            "repository"
        } else {
            "repositories"
        };
        println!(
            "🔄 Syncing {} {repo_word} ({concurrent_limit} concurrent)\n",
            repositories.len()
        );
        println!("{DIM}Phase 1/2 · pull{RESET}");
    }

    let pull_context = create_processing_context(
        std::sync::Arc::clone(&repositories),
        start_time,
        concurrent_limit,
    )?;
    let push_context = create_processing_context(
        std::sync::Arc::clone(&repositories),
        start_time,
        concurrent_limit,
    )?;
    let pull_run =
        process_pull_repositories(pull_context, true, verbose, show_changes, true, false).await;

    if verbose {
        println!("{DIM}Phase 2/2 · push{RESET}");
    }
    let push_run = process_push_repositories(
        push_context,
        auto_upstream,
        verbose,
        show_changes,
        no_drift_check,
        false,
    )
    .await;

    let (drift_count, drift_lines) = if no_drift_check {
        (0, Vec::new())
    } else {
        format_nested_drift_work_items()
    };
    let pull_stats = crate::core::acquire_stats_lock(&pull_run.statistics);
    let push_stats = crate::core::acquire_stats_lock(&push_run.statistics);
    let report = generate_sync_report(
        &pull_stats,
        &push_stats,
        start_time.elapsed(),
        repositories.len(),
        show_changes,
        drift_count,
        &drift_lines,
    );
    drop(push_stats);
    drop(pull_stats);
    println!("{report}\n");
    set_terminal_title_and_flush("✅ repos sync");

    let total_errors = pull_run.error_count + push_run.error_count;
    if total_errors > 0 {
        anyhow::bail!(
            "sync failed in {total_errors} operations ({} pull, {} push)",
            pull_run.error_count,
            push_run.error_count
        );
    }
    Ok(())
}

/// Fetches remote references across every discovered repository without
/// changing local branches or worktrees.
pub async fn handle_fetch_command(
    verbose: bool,
    jobs: Option<usize>,
    sequential: bool,
) -> Result<()> {
    let Some(context) =
        prepare_transfer_context(FleetTransfer::Fetch, verbose, jobs, sequential, "").await?
    else {
        return Ok(());
    };

    let run = process_fetch_repositories(context, verbose).await;
    set_terminal_title_and_flush(FleetTransfer::Fetch.completed_title());
    run.ensure_success(FleetTransfer::Fetch.command())
}

async fn process_fetch_repositories(
    context: crate::core::ProcessingContext,
    verbose: bool,
) -> TransferRun {
    use crate::core::acquire_stats_lock;
    use crate::git::operations::fetch_remote_updates;
    use futures::stream::{FuturesUnordered, StreamExt};

    let statistics = std::sync::Arc::clone(&context.statistics);
    let footer_message = statistics
        .lock()
        .unwrap()
        .generate_fetch_live_summary(context.total_repos);
    let (repo_progress_bars, footer_pb, single_pb) = create_sync_progress(
        &context,
        verbose,
        &format!("{DIM}fetching...{RESET}"),
        footer_message,
    );
    let start_time = context.start_time;

    let mut futures = FuturesUnordered::new();
    for ((repo_name, repo_path), progress_bar) in
        context.repositories.iter().zip(repo_progress_bars)
    {
        let semaphore = std::sync::Arc::clone(&context.semaphore);
        let stats = std::sync::Arc::clone(&context.statistics);
        let footer = footer_pb.clone();
        let single = single_pb.clone();
        let total_repos = context.total_repos;
        let max_name_length = context.max_name_length;

        futures.push(async move {
            use crate::core::config::SLOW_REPO_THRESHOLD_SECS;

            let started = std::time::Instant::now();
            let mut watchdog = spawn_slow_repo_watchdog(
                single.as_ref(),
                repo_name,
                std::time::Duration::from_secs(SLOW_REPO_THRESHOLD_SECS),
            );
            let _permit = match semaphore.acquire().await {
                Ok(permit) => permit,
                Err(error) => {
                    stop_slow_repo_watchdog(&mut watchdog).await;
                    record_semaphore_error(
                        "fetch",
                        repo_name,
                        repo_path,
                        &error,
                        &stats,
                        progress_bar.as_ref(),
                        single.as_ref(),
                    );
                    return;
                }
            };

            let result = fetch_remote_updates(repo_path).await;
            stop_slow_repo_watchdog(&mut watchdog).await;

            let elapsed = started.elapsed();
            let display_message = if elapsed.as_secs() >= SLOW_REPO_THRESHOLD_SECS {
                format!("{} ({:.1}s)", result.message, elapsed.as_secs_f32())
            } else {
                result.message.clone()
            };
            if verbose {
                if let Some(progress_bar) = progress_bar.as_ref() {
                    progress_bar.set_prefix(format!(
                        "{} {:width$}",
                        result.status.symbol(),
                        repo_name,
                        width = max_name_length
                    ));
                    progress_bar
                        .set_message(format!("{:<10}   {display_message}", result.status.text()));
                    progress_bar.finish();
                }
            } else if let Some(progress_bar) = single.as_ref() {
                progress_bar.set_message(format_live_repo_status(repo_name, result.status));
                progress_bar.inc(1);
            }

            let stats_guard = acquire_stats_lock(&stats);
            stats_guard.update_with_failure(
                repo_name,
                &repo_path.to_string_lossy(),
                &result.status,
                &result.message,
                false,
                result.failure.as_ref(),
            );
            if verbose {
                footer.set_message(stats_guard.generate_fetch_summary(start_time.elapsed()));
            }
            drop(stats_guard);
            if !verbose {
                footer.set_message(
                    stats
                        .lock()
                        .unwrap()
                        .generate_fetch_live_summary(total_repos),
                );
            }
        });
    }

    while futures.next().await.is_some() {}
    footer_pb.finish_and_clear();

    let final_stats = acquire_stats_lock(&statistics);
    println!(
        "{}",
        final_stats.generate_fetch_report(start_time.elapsed())
    );
    println!();
    let error_count = final_stats
        .error_repos
        .load(std::sync::atomic::Ordering::Relaxed);
    drop(final_stats);

    TransferRun {
        statistics,
        error_count,
    }
}

/// Handles the repository push command
pub async fn handle_push_command(
    auto_upstream: bool,
    verbose: bool,
    show_changes: bool,
    no_drift_check: bool,
    jobs: Option<usize>,
    sequential: bool,
) -> Result<()> {
    let Some(context) =
        prepare_transfer_context(FleetTransfer::Push, verbose, jobs, sequential, "").await?
    else {
        return Ok(());
    };

    let run = process_push_repositories(
        context,
        auto_upstream,
        verbose,
        show_changes,
        no_drift_check,
        true,
    )
    .await;

    set_terminal_title_and_flush(FleetTransfer::Push.completed_title());
    run.ensure_success(FleetTransfer::Push.command())
}

/// Processes all repositories with pipelined fetch+push for optimal performance
///
/// Each repository flows through: fetch → immediately push (no waiting for all fetches)
/// Fetch uses high concurrency (2x), push uses standard concurrency with rate limit protection
async fn process_push_repositories(
    context: crate::core::ProcessingContext,
    auto_upstream: bool,
    verbose: bool,
    show_changes: bool,
    no_drift_check: bool,
    render_report: bool,
) -> TransferRun {
    use crate::core::acquire_stats_lock;
    use crate::git::fetch_and_analyze;
    use crate::git::operations::push_if_needed_with_context;
    use futures::stream::{FuturesUnordered, StreamExt};

    // Use 2x concurrency for fetch phase (I/O bound), standard concurrency for push phase
    use crate::core::config::FETCH_CONCURRENT_CAP;
    let fetch_concurrency = (context.max_concurrency * 2).min(FETCH_CONCURRENT_CAP);
    let fetch_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(fetch_concurrency));
    let statistics = std::sync::Arc::clone(&context.statistics);

    let push_footer = context
        .statistics
        .lock()
        .unwrap()
        .generate_push_live_summary(context.total_repos);
    let (repo_progress_bars, footer_pb, single_pb) = create_sync_progress(
        &context,
        verbose,
        &format!("{DIM}processing...{RESET}"),
        push_footer,
    );

    let max_name_length = context.max_name_length;
    let start_time = context.start_time;

    // Track rate limit errors for adaptive backoff
    let rate_limit_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let has_rate_limit = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Create pipelined futures: each does fetch → immediately push
    let mut pipeline_futures = FuturesUnordered::new();
    for ((repo_name, repo_path), progress_bar) in
        context.repositories.iter().zip(repo_progress_bars)
    {
        let fetch_semaphore_clone = std::sync::Arc::clone(&fetch_semaphore);
        let push_semaphore_clone = std::sync::Arc::clone(&context.semaphore);
        let stats_clone = std::sync::Arc::clone(&context.statistics);
        let footer_clone = footer_pb.clone();
        let single_pb_clone = single_pb.clone();
        let rate_limit_count_clone = std::sync::Arc::clone(&rate_limit_count);
        let has_rate_limit_clone = std::sync::Arc::clone(&has_rate_limit);
        let verbose_clone = verbose;
        let max_name_length_clone = max_name_length;
        let start_time_clone = start_time;
        let total_repos_clone = context.total_repos;

        let future = async move {
            use crate::core::config::SLOW_REPO_THRESHOLD_SECS;

            // Track start time for this repo
            let repo_start_time = std::time::Instant::now();
            let mut slow_repo_watchdog = spawn_slow_repo_watchdog(
                single_pb_clone.as_ref(),
                repo_name,
                std::time::Duration::from_secs(SLOW_REPO_THRESHOLD_SECS),
            );

            // PHASE 1: Fetch with high concurrency
            let _fetch_permit = match fetch_semaphore_clone.acquire().await {
                Ok(permit) => permit,
                Err(e) => {
                    stop_slow_repo_watchdog(&mut slow_repo_watchdog).await;
                    record_semaphore_error(
                        "fetch",
                        repo_name,
                        repo_path,
                        &e,
                        &stats_clone,
                        progress_bar.as_ref(),
                        single_pb_clone.as_ref(),
                    );
                    return;
                }
            };
            let fetch_result = fetch_and_analyze(repo_path, auto_upstream).await;
            drop(_fetch_permit); // Fetch permit released here

            // PHASE 2: Push with standard concurrency + rate limit protection
            let _push_permit = match push_semaphore_clone.acquire().await {
                Ok(permit) => permit,
                Err(e) => {
                    stop_slow_repo_watchdog(&mut slow_repo_watchdog).await;
                    record_semaphore_error(
                        "push",
                        repo_name,
                        repo_path,
                        &e,
                        &stats_clone,
                        progress_bar.as_ref(),
                        single_pb_clone.as_ref(),
                    );
                    return;
                }
            };

            // Attempt push with retry on rate limit
            let mut attempt = 0;
            let max_attempts = 2;
            let result = loop {
                attempt += 1;
                let mut result =
                    push_if_needed_with_context(repo_path, &fetch_result, auto_upstream).await;

                // Check for rate limit error
                if result.message.contains("⚠️ RATE LIMIT") {
                    has_rate_limit_clone.store(true, std::sync::atomic::Ordering::Release);
                    rate_limit_count_clone.fetch_add(1, std::sync::atomic::Ordering::Release);

                    if attempt < max_attempts {
                        // Wait briefly and retry
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                        continue;
                    }
                    // Max attempts reached, return with suggestion
                    let suggestion = format!(
                        "{} (try reducing concurrency with --jobs N or --sequential)",
                        result.message.replace("⚠️ RATE LIMIT: ", "")
                    );
                    result.message.clone_from(&suggestion);
                    if let Some(failure) = &mut result.failure {
                        failure.message = suggestion;
                    }
                    break result;
                }

                break result;
            };

            stop_slow_repo_watchdog(&mut slow_repo_watchdog).await;

            let status = result.status;
            let message = &result.message;
            let has_uncommitted_changes = result.has_uncommitted;

            // Calculate elapsed time for this repo
            let repo_elapsed = repo_start_time.elapsed();
            let repo_elapsed_secs = repo_elapsed.as_secs_f32();

            let display_message =
                if has_uncommitted_changes && matches!(status, crate::git::Status::Synced) {
                    format!("{message} (uncommitted changes)")
                } else {
                    message.clone()
                };

            // Add elapsed time warning if repo took longer than threshold
            let display_message = if repo_elapsed.as_secs() >= SLOW_REPO_THRESHOLD_SECS {
                format!("{display_message} ({repo_elapsed_secs:.1}s)")
            } else {
                display_message
            };

            if verbose_clone {
                if let Some(progress_bar) = progress_bar.as_ref() {
                    progress_bar.set_prefix(format!(
                        "{} {:width$}",
                        status.symbol(),
                        repo_name,
                        width = max_name_length_clone
                    ));
                    progress_bar.set_message(format!(
                        "{:<10}   {}",
                        status.text(),
                        display_message
                    ));
                    progress_bar.finish();
                }
            } else if let Some(progress_bar) = single_pb_clone.as_ref() {
                progress_bar.set_message(format_live_repo_status(repo_name, status));
                progress_bar.inc(1);
            }

            let stats_guard = acquire_stats_lock(&stats_clone);
            stats_guard.update_with_failure(
                repo_name,
                &repo_path.to_string_lossy(),
                &status,
                message,
                has_uncommitted_changes,
                result.failure.as_ref(),
            );

            let duration = start_time_clone.elapsed();
            if verbose_clone {
                footer_clone.set_message(stats_guard.generate_push_summary(duration));
            }
            drop(stats_guard);
            if !verbose_clone {
                let stats_locked = stats_clone.lock().unwrap();
                footer_clone
                    .set_message(stats_locked.generate_push_live_summary(total_repos_clone));
            }
        };
        pipeline_futures.push(future);
    }

    while pipeline_futures.next().await.is_some() {}

    // Show rate limit warning if detected
    if has_rate_limit.load(std::sync::atomic::Ordering::Acquire) {
        let count = rate_limit_count.load(std::sync::atomic::Ordering::Acquire);
        eprintln!("\n⚠️  Rate limit detected on {count} operation(s).");
        eprintln!("💡 Try reducing concurrency: repos push --jobs 3");
    }

    footer_pb.finish_and_clear();

    let final_stats = acquire_stats_lock(&statistics);
    if render_report {
        let (drift_count, drift_lines) = if no_drift_check {
            (0, Vec::new())
        } else {
            format_nested_drift_work_items()
        };
        let report = if drift_count == 0 && drift_lines.is_empty() {
            final_stats.generate_push_report(context.start_time.elapsed(), show_changes)
        } else {
            final_stats.generate_push_report_with_follow_up(
                context.start_time.elapsed(),
                show_changes,
                drift_count,
                &drift_lines,
            )
        };
        println!("{report}");
        println!();
    }

    let error_count = final_stats
        .error_repos
        .load(std::sync::atomic::Ordering::Relaxed);
    drop(final_stats);
    TransferRun {
        statistics,
        error_count,
    }
}

fn format_nested_drift_work_items() -> (usize, Vec<String>) {
    crate::subrepo::status::analyze_subrepos_quiet()
        .map(|statuses| crate::subrepo::status::format_drift_work_items(&statuses))
        .unwrap_or_else(|_| (0, Vec::new()))
}

/// Handles the repository pull command
pub async fn handle_pull_command(
    use_rebase: bool,
    verbose: bool,
    show_changes: bool,
    no_drift_check: bool,
    jobs: Option<usize>,
    sequential: bool,
) -> Result<()> {
    let qualifier = if use_rebase { " with rebase" } else { "" };
    let Some(context) =
        prepare_transfer_context(FleetTransfer::Pull, verbose, jobs, sequential, qualifier).await?
    else {
        return Ok(());
    };

    let run = process_pull_repositories(
        context,
        use_rebase,
        verbose,
        show_changes,
        no_drift_check,
        true,
    )
    .await;

    set_terminal_title_and_flush(FleetTransfer::Pull.completed_title());
    run.ensure_success(FleetTransfer::Pull.command())
}

/// Processes all repositories with pipelined fetch+pull for optimal performance
///
/// Each repository flows through: fetch → immediately pull (no waiting for all fetches)
/// Fetch uses high concurrency (2x), pull uses standard concurrency with rate limit protection
async fn process_pull_repositories(
    context: crate::core::ProcessingContext,
    use_rebase: bool,
    verbose: bool,
    show_changes: bool,
    no_drift_check: bool,
    render_report: bool,
) -> TransferRun {
    use crate::core::acquire_stats_lock;
    use crate::git::fetch_and_analyze_for_pull;
    use crate::git::operations::pull_if_needed_with_context;
    use futures::stream::{FuturesUnordered, StreamExt};

    // Use 2x concurrency for fetch phase (I/O bound), standard concurrency for pull phase
    use crate::core::config::FETCH_CONCURRENT_CAP;
    let fetch_concurrency = (context.max_concurrency * 2).min(FETCH_CONCURRENT_CAP);
    let fetch_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(fetch_concurrency));
    let statistics = std::sync::Arc::clone(&context.statistics);

    let pull_footer = context
        .statistics
        .lock()
        .unwrap()
        .generate_pull_live_summary(context.total_repos);
    let (repo_progress_bars, footer_pb, single_pb) =
        create_sync_progress(&context, verbose, "🔽 Processing...", pull_footer);

    let max_name_length = context.max_name_length;
    let start_time = context.start_time;

    // Track rate limit errors for adaptive backoff
    let rate_limit_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let has_rate_limit = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Create pipelined futures: each does fetch → immediately pull
    let mut pipeline_futures = FuturesUnordered::new();
    for ((repo_name, repo_path), progress_bar) in
        context.repositories.iter().zip(repo_progress_bars)
    {
        let fetch_semaphore_clone = std::sync::Arc::clone(&fetch_semaphore);
        let pull_semaphore_clone = std::sync::Arc::clone(&context.semaphore);
        let stats_clone = std::sync::Arc::clone(&context.statistics);
        let footer_clone = footer_pb.clone();
        let single_pb_clone = single_pb.clone();
        let rate_limit_count_clone = std::sync::Arc::clone(&rate_limit_count);
        let has_rate_limit_clone = std::sync::Arc::clone(&has_rate_limit);
        let verbose_clone = verbose;
        let max_name_length_clone = max_name_length;
        let start_time_clone = start_time;
        let total_repos_clone = context.total_repos;

        let future = async move {
            use crate::core::config::SLOW_REPO_THRESHOLD_SECS;

            // Track start time for this repo
            let repo_start_time = std::time::Instant::now();

            // PHASE 1: Fetch with high concurrency
            let _fetch_permit = match fetch_semaphore_clone.acquire().await {
                Ok(permit) => permit,
                Err(e) => {
                    record_semaphore_error(
                        "fetch",
                        repo_name,
                        repo_path,
                        &e,
                        &stats_clone,
                        progress_bar.as_ref(),
                        single_pb_clone.as_ref(),
                    );
                    return;
                }
            };
            let fetch_result = fetch_and_analyze_for_pull(repo_path).await;
            drop(_fetch_permit); // Fetch permit released here

            // PHASE 2: Pull with standard concurrency + rate limit protection
            let _pull_permit = match pull_semaphore_clone.acquire().await {
                Ok(permit) => permit,
                Err(e) => {
                    record_semaphore_error(
                        "pull",
                        repo_name,
                        repo_path,
                        &e,
                        &stats_clone,
                        progress_bar.as_ref(),
                        single_pb_clone.as_ref(),
                    );
                    return;
                }
            };

            // Attempt pull with retry on rate limit
            let mut attempt = 0;
            let max_attempts = 2;
            let result = loop {
                attempt += 1;
                let mut result =
                    pull_if_needed_with_context(repo_path, &fetch_result, use_rebase).await;

                // Check for rate limit error
                if result.message.contains("⚠️ RATE LIMIT") {
                    has_rate_limit_clone.store(true, std::sync::atomic::Ordering::Release);
                    rate_limit_count_clone.fetch_add(1, std::sync::atomic::Ordering::Release);

                    if attempt < max_attempts {
                        // Wait briefly and retry
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                        continue;
                    }
                    // Max attempts reached, return with suggestion
                    let suggestion = format!(
                        "{} (try reducing concurrency with --jobs N or --sequential)",
                        result.message.replace("⚠️ RATE LIMIT: ", "")
                    );
                    result.message.clone_from(&suggestion);
                    if let Some(failure) = &mut result.failure {
                        failure.message = suggestion;
                    }
                    break result;
                }

                break result;
            };

            let status = result.status;
            let message = &result.message;
            let has_uncommitted_changes = result.has_uncommitted;

            // Calculate elapsed time for this repo
            let repo_elapsed = repo_start_time.elapsed();
            let repo_elapsed_secs = repo_elapsed.as_secs_f32();

            let display_message =
                if has_uncommitted_changes && matches!(status, crate::git::Status::Synced) {
                    format!("{message} (uncommitted changes)")
                } else {
                    message.clone()
                };

            // Add elapsed time warning if repo took longer than threshold
            let display_message = if repo_elapsed.as_secs() >= SLOW_REPO_THRESHOLD_SECS {
                format!("{display_message} ({repo_elapsed_secs:.1}s)")
            } else {
                display_message
            };

            if verbose_clone {
                if let Some(progress_bar) = progress_bar.as_ref() {
                    progress_bar.set_prefix(format!(
                        "{} {:width$}",
                        status.symbol(),
                        repo_name,
                        width = max_name_length_clone
                    ));
                    progress_bar.set_message(format!(
                        "{:<10}   {}",
                        status.text(),
                        display_message
                    ));
                    progress_bar.finish();
                }
            } else if let Some(progress_bar) = single_pb_clone.as_ref() {
                progress_bar.set_message(format_live_repo_status(repo_name, status));
                progress_bar.inc(1);
            }

            let stats_guard = acquire_stats_lock(&stats_clone);
            stats_guard.update_with_failure(
                repo_name,
                &repo_path.to_string_lossy(),
                &status,
                message,
                has_uncommitted_changes,
                result.failure.as_ref(),
            );

            let duration = start_time_clone.elapsed();
            if verbose_clone {
                footer_clone.set_message(stats_guard.generate_pull_summary(duration));
            }
            drop(stats_guard);
            if !verbose_clone {
                let stats_locked = stats_clone.lock().unwrap();
                footer_clone
                    .set_message(stats_locked.generate_pull_live_summary(total_repos_clone));
            }
        };
        pipeline_futures.push(future);
    }

    while pipeline_futures.next().await.is_some() {}

    // Show rate limit warning if detected
    if has_rate_limit.load(std::sync::atomic::Ordering::Acquire) {
        let count = rate_limit_count.load(std::sync::atomic::Ordering::Acquire);
        eprintln!("\n⚠️  Rate limit detected on {count} operation(s).");
        eprintln!("💡 Try reducing concurrency: repos pull --jobs 3");
    }

    footer_pb.finish_and_clear();

    let final_stats = acquire_stats_lock(&statistics);
    if render_report {
        let (drift_count, drift_lines) = if no_drift_check {
            (0, Vec::new())
        } else {
            format_nested_drift_work_items()
        };
        let report = if drift_count == 0 && drift_lines.is_empty() {
            final_stats.generate_pull_report(context.start_time.elapsed(), show_changes)
        } else {
            final_stats.generate_pull_report_with_follow_up(
                context.start_time.elapsed(),
                show_changes,
                drift_count,
                &drift_lines,
            )
        };
        println!("{report}");
        println!();
    }

    let error_count = final_stats
        .error_repos
        .load(std::sync::atomic::Ordering::Relaxed);
    drop(final_stats);
    TransferRun {
        statistics,
        error_count,
    }
}

#[cfg(test)]
mod tests {
    use super::{spawn_slow_repo_watchdog, stop_slow_repo_watchdog};
    use indicatif::ProgressBar;
    use std::time::Duration;

    #[tokio::test]
    async fn slow_repo_watchdog_names_the_active_repository() {
        let progress_bar = ProgressBar::hidden();
        let mut watchdog =
            spawn_slow_repo_watchdog(Some(&progress_bar), "slow-repo", Duration::from_millis(1));

        watchdog
            .take()
            .expect("watchdog should start")
            .await
            .expect("watchdog should complete");

        assert_eq!(progress_bar.message(), "slow-repo · still running...");
    }

    #[tokio::test]
    async fn stopped_watchdog_cannot_overwrite_the_final_status() {
        let progress_bar = ProgressBar::hidden();
        let mut watchdog =
            spawn_slow_repo_watchdog(Some(&progress_bar), "slow-repo", Duration::from_secs(60));

        stop_slow_repo_watchdog(&mut watchdog).await;
        progress_bar.set_message("complete");
        tokio::task::yield_now().await;

        assert!(watchdog.is_none());
        assert_eq!(progress_bar.message(), "complete");
    }

    #[test]
    fn watchdog_is_disabled_without_a_progress_bar() {
        assert!(spawn_slow_repo_watchdog(None, "repo", Duration::ZERO).is_none());
    }
}
