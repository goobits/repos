//! Parent-first fleet pull pipeline.

use super::progress::{
    create_sync_progress, finish_sync_progress, record_semaphore_error, record_transfer_result,
    TransferDirection, TransferResultContext,
};
use super::{prepare_transfer_context, FleetTransfer, TransferRun};
use crate::core::{
    print_final_report, set_terminal_title_and_flush, RepositoryOrder, TopologySnapshot,
};
use crate::git::failure::GitOperationResult;
use anyhow::Result;

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
        None,
    )
    .await;
    set_terminal_title_and_flush(FleetTransfer::Pull.completed_title());
    run.ensure_success(FleetTransfer::Pull.command())
}

pub(super) async fn process_pull_repositories(
    context: crate::core::ProcessingContext,
    use_rebase: bool,
    verbose: bool,
    show_changes: bool,
    no_drift_check: bool,
    render_report: bool,
    topology: Option<std::sync::Arc<TopologySnapshot>>,
) -> TransferRun {
    use crate::core::config::FETCH_CONCURRENT_CAP;
    use crate::git::fetch_and_analyze_for_pull;
    use futures::stream::{FuturesUnordered, StreamExt};

    let fetch_concurrency = context
        .max_concurrency
        .saturating_mul(2)
        .min(FETCH_CONCURRENT_CAP);
    let fetch_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(fetch_concurrency));
    let statistics = std::sync::Arc::clone(&context.statistics);
    let footer_message = context
        .statistics
        .generate_pull_live_summary(context.total_repos);
    let (repository_bars, footer, concise) =
        create_sync_progress(&context, verbose, "🔽 Processing...", footer_message);
    let start_time = context.start_time;
    let rate_limit_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let has_rate_limit = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    let topology = topology
        .unwrap_or_else(|| std::sync::Arc::new(TopologySnapshot::new(&context.repositories)));
    for wave in topology.topology().waves(RepositoryOrder::ParentsFirst) {
        let mut futures = FuturesUnordered::new();
        for index in wave {
            let (repository, path) = &context.repositories[index];
            let progress_bar = repository_bars[index].clone();
            let fetch_semaphore = std::sync::Arc::clone(&fetch_semaphore);
            let pull_semaphore = std::sync::Arc::clone(&context.semaphore);
            let statistics = std::sync::Arc::clone(&context.statistics);
            let footer = footer.clone();
            let concise = concise.clone();
            let rate_limit_count = std::sync::Arc::clone(&rate_limit_count);
            let has_rate_limit = std::sync::Arc::clone(&has_rate_limit);
            let max_name_length = context.max_name_length;
            let total_repositories = context.total_repos;

            futures.push(async move {
                let started = std::time::Instant::now();
                let fetch_permit = match fetch_semaphore.acquire().await {
                    Ok(permit) => permit,
                    Err(error) => {
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
                let fetch_result = fetch_and_analyze_for_pull(path).await;
                drop(fetch_permit);

                let pull_permit = match pull_semaphore.acquire().await {
                    Ok(permit) => permit,
                    Err(error) => {
                        record_semaphore_error(
                            "pull",
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
                let result = pull_with_rate_limit_retry(
                    path,
                    &fetch_result,
                    use_rebase,
                    &has_rate_limit,
                    &rate_limit_count,
                )
                .await;
                drop(pull_permit);

                record_transfer_result(
                    TransferResultContext {
                        repository,
                        path,
                        verbose,
                        max_name_length,
                        repository_bar: progress_bar.as_ref(),
                        concise_bar: concise.as_ref(),
                        statistics: &statistics,
                        footer: &footer,
                        start_time,
                        total_repositories,
                    },
                    result,
                    started.elapsed(),
                    TransferDirection::Pull,
                );
            });
        }
        while futures.next().await.is_some() {}
    }

    if has_rate_limit.load(std::sync::atomic::Ordering::Acquire) {
        let count = rate_limit_count.load(std::sync::atomic::Ordering::Acquire);
        eprintln!("\n⚠️  Rate limit detected on {count} operation(s).");
        eprintln!("💡 Try reducing concurrency: repos pull --jobs 3");
    }
    finish_sync_progress(&footer, concise.as_ref());

    let (drift_count, drift_lines) = if render_report && !no_drift_check {
        super::format_nested_drift_work_items_with_topology(&context.repositories, &topology)
    } else {
        (0, Vec::new())
    };
    let final_stats = statistics.as_ref();
    if render_report {
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
        print_final_report(&report);
    }
    let error_count = final_stats
        .error_repos
        .load(std::sync::atomic::Ordering::Relaxed);
    TransferRun {
        statistics,
        error_count,
    }
}

async fn pull_with_rate_limit_retry(
    path: &std::path::Path,
    fetch_result: &crate::git::operations::PullFetchResult,
    use_rebase: bool,
    has_rate_limit: &std::sync::atomic::AtomicBool,
    rate_limit_count: &std::sync::atomic::AtomicUsize,
) -> GitOperationResult {
    use crate::git::operations::pull_if_needed_with_context;

    let mut attempt = 0;
    loop {
        attempt += 1;
        let mut result = pull_if_needed_with_context(path, fetch_result, use_rebase).await;
        if !result.message.contains("⚠️ RATE LIMIT") {
            return result;
        }
        has_rate_limit.store(true, std::sync::atomic::Ordering::Release);
        rate_limit_count.fetch_add(1, std::sync::atomic::Ordering::Release);
        if attempt < 2 {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            continue;
        }
        let suggestion = format!(
            "{} (try reducing concurrency with --jobs N or --sequential)",
            result.message.replace("⚠️ RATE LIMIT: ", "")
        );
        result.message.clone_from(&suggestion);
        if let Some(failure) = &mut result.failure {
            failure.message = suggestion;
        }
        return result;
    }
}
