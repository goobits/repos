//! Progress-bar and watchdog helpers shared by transfer pipelines.

use crate::git::failure::GitOperationResult;
use crate::git::Status;

const RESET: &str = "\x1b[0m";
const GREEN: &str = "\x1b[1;38;5;114m";
const YELLOW: &str = "\x1b[1;38;5;221m";
const RED: &str = "\x1b[1;38;5;203m";
const DIM: &str = "\x1b[2m";

pub(super) type ProgressBars = (
    Vec<Option<indicatif::ProgressBar>>,
    indicatif::ProgressBar,
    Option<indicatif::ProgressBar>,
);

pub(super) fn format_live_repo_status(repo_name: &str, status: Status) -> String {
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

pub(super) fn create_sync_progress(
    context: &crate::core::ProcessingContext,
    verbose: bool,
    concise_message: &str,
    footer_message: String,
) -> ProgressBars {
    use indicatif::{ProgressBar, ProgressStyle};

    let (repository_bars, single_bar) = if verbose {
        let bars = context
            .repositories
            .iter()
            .map(|(repository, _)| {
                let bar = crate::core::create_progress_bar(
                    &context.multi_progress,
                    &context.progress_style,
                    repository,
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
    (repository_bars, footer, single_bar)
}

pub(super) fn finish_sync_progress(
    footer: &indicatif::ProgressBar,
    concise: Option<&indicatif::ProgressBar>,
) {
    if let Some(concise) = concise {
        concise.finish();
    }
    footer.finish_and_clear();
}

pub(super) fn record_semaphore_error(
    operation: &str,
    repository: &str,
    path: &std::path::Path,
    error: &tokio::sync::AcquireError,
    statistics: &std::sync::Arc<std::sync::Mutex<crate::core::SyncStatistics>>,
    repository_bar: Option<&indicatif::ProgressBar>,
    single_bar: Option<&indicatif::ProgressBar>,
) {
    eprintln!("Error: Failed to acquire {operation} permit for {repository}: {error}");
    let message = format!("semaphore error: {error}");
    crate::core::acquire_stats_lock(statistics).update(
        repository,
        &path.to_string_lossy(),
        &Status::Error,
        &message,
        false,
    );

    if let Some(bar) = repository_bar {
        bar.finish_with_message(format!("🔴 {repository}  semaphore error"));
    } else if let Some(bar) = single_bar {
        bar.set_message(format_live_repo_status(repository, Status::Error));
        bar.inc(1);
    }
}

pub(super) fn spawn_slow_repo_watchdog(
    progress_bar: Option<&indicatif::ProgressBar>,
    repository: &str,
    delay: std::time::Duration,
) -> Option<tokio::task::JoinHandle<()>> {
    progress_bar.map(|progress_bar| {
        let progress_bar = progress_bar.clone();
        let repository = repository.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            progress_bar.set_message(format!("{repository} · still running..."));
        })
    })
}

pub(super) async fn stop_slow_repo_watchdog(watchdog: &mut Option<tokio::task::JoinHandle<()>>) {
    if let Some(watchdog) = watchdog.take() {
        watchdog.abort();
        let _ = watchdog.await;
    }
}

#[derive(Clone, Copy)]
pub(super) enum TransferDirection {
    Push,
    Pull,
}

pub(super) struct TransferResultContext<'a> {
    pub repository: &'a str,
    pub path: &'a std::path::Path,
    pub verbose: bool,
    pub max_name_length: usize,
    pub repository_bar: Option<&'a indicatif::ProgressBar>,
    pub concise_bar: Option<&'a indicatif::ProgressBar>,
    pub statistics: &'a std::sync::Arc<std::sync::Mutex<crate::core::SyncStatistics>>,
    pub footer: &'a indicatif::ProgressBar,
    pub start_time: std::time::Instant,
    pub total_repositories: usize,
}

pub(super) fn record_transfer_result(
    context: TransferResultContext<'_>,
    result: GitOperationResult,
    elapsed: std::time::Duration,
    direction: TransferDirection,
) {
    use crate::core::acquire_stats_lock;
    use crate::core::config::SLOW_REPO_THRESHOLD_SECS;

    let status = result.status;
    let message = &result.message;
    let display_message = if result.has_uncommitted && matches!(status, Status::Synced) {
        format!("{message} (uncommitted changes)")
    } else {
        message.clone()
    };
    let display_message = if elapsed.as_secs() >= SLOW_REPO_THRESHOLD_SECS {
        format!("{display_message} ({:.1}s)", elapsed.as_secs_f32())
    } else {
        display_message
    };

    if context.verbose {
        if let Some(progress_bar) = context.repository_bar {
            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                context.repository,
                width = context.max_name_length
            ));
            progress_bar.set_message(format!("{:<10}   {}", status.text(), display_message));
            progress_bar.finish();
        }
    } else if let Some(progress_bar) = context.concise_bar {
        progress_bar.set_message(format_live_repo_status(context.repository, status));
        progress_bar.inc(1);
    }

    let statistics = acquire_stats_lock(context.statistics);
    statistics.update_with_failure(
        context.repository,
        &context.path.to_string_lossy(),
        &status,
        message,
        result.has_uncommitted,
        result.failure.as_ref(),
    );
    if context.verbose {
        let summary = match direction {
            TransferDirection::Push => {
                statistics.generate_push_summary(context.start_time.elapsed())
            }
            TransferDirection::Pull => {
                statistics.generate_pull_summary(context.start_time.elapsed())
            }
        };
        context.footer.set_message(summary);
    }
    drop(statistics);
    if !context.verbose {
        let statistics = context.statistics.lock().unwrap();
        let summary = match direction {
            TransferDirection::Push => {
                statistics.generate_push_live_summary(context.total_repositories)
            }
            TransferDirection::Pull => {
                statistics.generate_pull_live_summary(context.total_repositories)
            }
        };
        context.footer.set_message(summary);
    }
}
