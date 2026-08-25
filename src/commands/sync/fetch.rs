//! Fetch-only fleet transfer pipeline.

use super::progress::{
    create_sync_progress, finish_sync_progress, record_semaphore_error, spawn_slow_repo_watchdog,
    stop_slow_repo_watchdog,
};
use super::{prepare_transfer_context, FleetTransfer, TransferRun};
use crate::core::{print_final_report, set_terminal_title_and_flush};
use anyhow::Result;

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";

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
    let (repository_bars, footer, concise) = create_sync_progress(
        &context,
        verbose,
        &format!("{DIM}fetching...{RESET}"),
        footer_message,
    );
    let start_time = context.start_time;

    let mut futures = FuturesUnordered::new();
    for ((repository, path), progress_bar) in context.repositories.iter().zip(repository_bars) {
        let semaphore = std::sync::Arc::clone(&context.semaphore);
        let statistics = std::sync::Arc::clone(&context.statistics);
        let footer = footer.clone();
        let concise = concise.clone();
        let total_repositories = context.total_repos;
        let max_name_length = context.max_name_length;

        futures.push(async move {
            use crate::core::config::SLOW_REPO_THRESHOLD_SECS;

            let started = std::time::Instant::now();
            let mut watchdog = spawn_slow_repo_watchdog(
                concise.as_ref(),
                repository,
                std::time::Duration::from_secs(SLOW_REPO_THRESHOLD_SECS),
            );
            let _permit = match semaphore.acquire().await {
                Ok(permit) => permit,
                Err(error) => {
                    stop_slow_repo_watchdog(&mut watchdog).await;
                    record_semaphore_error(
                        "fetch",
                        repository,
                        path,
                        &error,
                        &statistics,
                        progress_bar.as_ref(),
                        concise.as_ref(),
                    );
                    return;
                }
            };

            let result = fetch_remote_updates(path).await;
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
                        repository,
                        width = max_name_length
                    ));
                    progress_bar
                        .set_message(format!("{:<10}   {display_message}", result.status.text()));
                    progress_bar.finish();
                }
            } else if let Some(progress_bar) = concise.as_ref() {
                progress_bar.set_message(super::progress::format_live_repo_status(
                    repository,
                    result.status,
                ));
                progress_bar.inc(1);
            }

            let stats = acquire_stats_lock(&statistics);
            stats.update_with_failure(
                repository,
                &path.to_string_lossy(),
                &result.status,
                &result.message,
                false,
                result.failure.as_ref(),
            );
            if verbose {
                footer.set_message(stats.generate_fetch_summary(start_time.elapsed()));
            }
            drop(stats);
            if !verbose {
                footer.set_message(
                    statistics
                        .lock()
                        .unwrap()
                        .generate_fetch_live_summary(total_repositories),
                );
            }
        });
    }

    while futures.next().await.is_some() {}
    finish_sync_progress(&footer, concise.as_ref());

    let final_stats = acquire_stats_lock(&statistics);
    print_final_report(&final_stats.generate_fetch_report(start_time.elapsed()));
    let error_count = final_stats
        .error_repos
        .load(std::sync::atomic::Ordering::Relaxed);
    drop(final_stats);

    TransferRun {
        statistics,
        error_count,
    }
}
