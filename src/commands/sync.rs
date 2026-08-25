//! Fleet fetch, pull, push, and two-way synchronization commands.
//!
//! Each directional transfer owns its execution pipeline in a child module;
//! this facade keeps shared setup and the two-phase sync orchestration.

mod fetch;
mod progress;
mod pull;
mod push;

#[cfg(test)]
mod tests;

use anyhow::Result;
use pull::process_pull_repositories;
use push::process_push_repositories;

use crate::core::{
    create_processing_context, generate_sync_report, init_command, print_final_report,
    set_terminal_title, set_terminal_title_and_flush, NO_REPOS_MESSAGE,
};

pub use fetch::handle_fetch_command;
pub use pull::handle_pull_command;
pub use push::handle_push_command;

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";
const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";

pub(super) struct TransferRun {
    pub(super) statistics: std::sync::Arc<std::sync::Mutex<crate::core::SyncStatistics>>,
    pub(super) error_count: u64,
}

#[derive(Clone, Copy)]
pub(super) enum FleetTransfer {
    Fetch,
    Push,
    Pull,
}

impl FleetTransfer {
    pub(super) fn command(self) -> &'static str {
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

    pub(super) fn completed_title(self) -> &'static str {
        match self {
            Self::Fetch => "✅ repos fetch",
            Self::Push => "✅ repos push",
            Self::Pull => "✅ repos pull",
        }
    }
}

impl TransferRun {
    pub(super) fn ensure_success(&self, operation: &str) -> Result<()> {
        if self.error_count > 0 {
            anyhow::bail!("{} repositories failed to {operation}", self.error_count);
        }
        Ok(())
    }
}

pub(super) async fn prepare_transfer_context(
    operation: FleetTransfer,
    verbose: bool,
    jobs: Option<usize>,
    sequential: bool,
    qualifier: &str,
) -> Result<Option<crate::core::ProcessingContext>> {
    use crate::core::config::get_git_concurrency;

    set_terminal_title(operation.running_title());
    let (start_time, repositories) = init_command(SCANNING_MESSAGE).await;
    println!();

    if repositories.is_empty() {
        println!("\r{NO_REPOS_MESSAGE}");
        set_terminal_title_and_flush(operation.completed_title());
        return Ok(None);
    }

    let concurrent_limit = get_git_concurrency(jobs, sequential);
    if verbose {
        let repository_label = if repositories.len() == 1 {
            "repository"
        } else {
            "repositories"
        };
        println!(
            "{} {} {repository_label}{qualifier} ({concurrent_limit} concurrent)\n",
            operation.activity(),
            repositories.len()
        );
    }

    create_processing_context(
        std::sync::Arc::new(repositories),
        start_time,
        concurrent_limit,
    )
    .inspect_err(|_| {
        set_terminal_title_and_flush(operation.completed_title());
    })
    .map(Some)
}

/// Pull safe remote changes, then push local commits against one fleet snapshot.
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
    let (start_time, repositories) = init_command(SCANNING_MESSAGE).await;
    println!();

    if repositories.is_empty() {
        println!("\r{NO_REPOS_MESSAGE}");
        set_terminal_title_and_flush("✅ repos sync");
        return Ok(());
    }

    let concurrent_limit = get_git_concurrency(jobs, sequential);
    let repositories = std::sync::Arc::new(repositories);
    if verbose {
        let repository_label = if repositories.len() == 1 {
            "repository"
        } else {
            "repositories"
        };
        println!(
            "🔄 Syncing {} {repository_label} ({concurrent_limit} concurrent)\n",
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
        format_nested_drift_work_items(&repositories)
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
    print_final_report(&report);
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

pub(super) fn format_nested_drift_work_items(
    repositories: &[(String, std::path::PathBuf)],
) -> (usize, Vec<String>) {
    format_nested_drift_result(
        crate::subrepo::status::analyze_nested_status_for_repositories(repositories),
    )
}

fn format_nested_drift_result(
    result: anyhow::Result<crate::subrepo::status::NestedStatusReport>,
) -> (usize, Vec<String>) {
    match result {
        Ok(report) => crate::subrepo::status::format_drift_work_items_with_inventory(&report),
        Err(error) => (1, crate::subrepo::status::format_drift_failure(&error)),
    }
}
