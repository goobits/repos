//! Repository push command implementation
//!
//! This module handles the core push functionality - discovering repositories
//! and pushing any unpushed commits to their upstream remotes.

use anyhow::Result;

use crate::core::{
    create_processing_context, init_command, set_terminal_title, set_terminal_title_and_flush,
    NO_REPOS_MESSAGE,
};
use crate::core::sync::{Stage, SyncCoordinator, SyncMode};
use crate::git::Status;

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";

/// Handles the repository push command
pub async fn handle_push_command(
    force_push: bool,
    verbose: bool,
    show_changes: bool,
    no_drift_check: bool,
    jobs: Option<usize>,
    sequential: bool,
) -> Result<()> {
    use crate::core::config::get_git_concurrency;

    // Set terminal title to indicate repos is running
    set_terminal_title("🚀 repos");

    let (start_time, repos) = init_command(SCANNING_MESSAGE).await;

    if repos.is_empty() {
        println!("\r{NO_REPOS_MESSAGE}");
        // Set terminal title to green checkbox to indicate completion
        set_terminal_title_and_flush("✅ repos");
        return Ok(());
    }

    // Determine concurrency level based on CLI args and system resources
    let concurrent_limit = get_git_concurrency(jobs, sequential);

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    let concurrency_info = if verbose {
        format!(" ({concurrent_limit} concurrent)")
    } else {
        String::new()
    };
    if verbose {
        print!(
            "\r🚀 Pushing {total_repos} {repo_word}{concurrency_info}                    \n"
        );
        println!();
    }

    // Create processing context with configured concurrency
    let context = match create_processing_context(std::sync::Arc::new(repos), start_time, concurrent_limit) {
        Ok(context) => context,
        Err(e) => {
            // If context creation fails, set completion title and return error
            set_terminal_title_and_flush("✅ repos");
            return Err(e);
        }
    };

    // Process all repositories concurrently
    process_push_repositories(context, force_push, verbose, show_changes).await;

    // Check for subrepo drift unless explicitly skipped
    if !no_drift_check {
        check_and_display_drift();
    }

    // Set terminal title to green checkbox to indicate completion
    set_terminal_title_and_flush("✅ repos");

    Ok(())
}

/// Processes all repositories with pipelined fetch+push for optimal performance
///
/// Each repository flows through: fetch → immediately push (no waiting for all fetches)
/// Fetch uses high concurrency (2x), push uses standard concurrency with rate limit protection
async fn process_push_repositories(
    context: crate::core::ProcessingContext,
    force_push: bool,
    verbose: bool,
    show_changes: bool,
) {
    use crate::core::{acquire_stats_lock, create_progress_bar};
    use crate::git::{fetch_and_analyze, push_if_needed};
    use futures::stream::{FuturesUnordered, StreamExt};

    let use_hud = !verbose;

    // Use 2x concurrency for fetch phase (I/O bound), standard concurrency for push phase
    use crate::core::config::FETCH_CONCURRENT_CAP;
    let fetch_concurrency = (context.max_concurrency * 2).min(FETCH_CONCURRENT_CAP);
    let fetch_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(fetch_concurrency));

    let coordinator = if use_hud {
        let repo_names: Vec<String> = context
            .repositories
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        Some(std::sync::Arc::new(SyncCoordinator::new(
            SyncMode::Push,
            &repo_names,
            context.total_repos,
            fetch_concurrency,
            context.max_concurrency,
            std::sync::Arc::clone(&context.statistics),
        )))
    } else {
        None
    };
    let (hud_stop_tx, hud_handle) = if let Some(coord) = &coordinator {
        let (stop_tx, handle) = coord.start();
        (Some(stop_tx), Some(handle))
    } else {
        (None, None)
    };

    let repo_progress_bars: Vec<Option<indicatif::ProgressBar>> = if verbose {
        context
            .repositories
            .iter()
            .map(|(repo_name, _)| {
                let pb = create_progress_bar(
                    &context.multi_progress,
                    &context.progress_style,
                    repo_name,
                );
                pb.set_message("processing...");
                Some(pb)
            })
            .collect()
    } else {
        vec![None; context.repositories.len()]
    };

    let footer_pb = if verbose {
        let _separator_pb = crate::core::create_separator_progress_bar(&context.multi_progress);
        let footer_pb = crate::core::create_footer_progress_bar(&context.multi_progress);
        footer_pb.set_message(
            "✅ 0 Pushed  🟢 0 Synced  🔴 0 Failed  🟡 0 No Upstream  🟠 0 Skipped".to_string(),
        );
        let _separator_pb2 = crate::core::create_separator_progress_bar(&context.multi_progress);
        Some(footer_pb)
    } else {
        None
    };

    let max_name_length = context.max_name_length;
    let start_time = context.start_time;
    let total_repos = context.total_repos;

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
        let rate_limit_count_clone = std::sync::Arc::clone(&rate_limit_count);
        let has_rate_limit_clone = std::sync::Arc::clone(&has_rate_limit);
        let coordinator_clone = coordinator.clone();
        let verbose_clone = verbose;
        let max_name_length_clone = max_name_length;
        let start_time_clone = start_time;
        let total_repos_clone = total_repos;

        let future = async move {
            use crate::core::config::SLOW_REPO_THRESHOLD_SECS;

            // Track start time for this repo
            let repo_start_time = std::time::Instant::now();

            // PHASE 1: Fetch with high concurrency
            let _fetch_permit = match fetch_semaphore_clone.acquire().await {
                Ok(permit) => permit,
                Err(e) => {
                    eprintln!(
                        "Error: Failed to acquire fetch permit for {repo_name}: {e}"
                    );

                    // Update statistics to track this failure
                    let stats_guard = acquire_stats_lock(&stats_clone);
                    stats_guard.update(
                        &repo_name,
                        &repo_path.to_string_lossy(),
                        &Status::Error,
                        &format!("semaphore error: {e}"),
                        false,
                    );

                    // Finish progress bar
                    if verbose_clone {
                        if let Some(progress_bar) = progress_bar.as_ref() {
                            progress_bar
                                .finish_with_message(format!("🔴 {repo_name}  semaphore error"));
                        }
                    }
                    if let Some(coordinator) = coordinator_clone.as_ref() {
                        coordinator.set_status(
                            &repo_name,
                            Status::Error,
                            &format!("semaphore error: {e}"),
                        );
                    }
                    return;
                }
            };
            if let Some(coordinator) = coordinator_clone.as_ref() {
                coordinator.set_stage(&repo_name, Stage::Checking, "git fetch --quiet");
            }
            let fetch_result = fetch_and_analyze(&repo_path, force_push).await;
            drop(_fetch_permit); // Fetch permit released here

            // PHASE 2: Push with standard concurrency + rate limit protection
            if let Some(coordinator) = coordinator_clone.as_ref() {
                coordinator.set_stage(&repo_name, Stage::Waiting, "waiting on upload slot");
            }
            let _push_permit = match push_semaphore_clone.acquire().await {
                Ok(permit) => permit,
                Err(e) => {
                    eprintln!(
                        "Error: Failed to acquire push permit for {repo_name}: {e}"
                    );

                    // Update statistics to track this failure
                    let stats_guard = acquire_stats_lock(&stats_clone);
                    stats_guard.update(
                        &repo_name,
                        &repo_path.to_string_lossy(),
                        &Status::Error,
                        &format!("semaphore error: {e}"),
                        false,
                    );

                    // Finish progress bar
                    if verbose_clone {
                        if let Some(progress_bar) = progress_bar.as_ref() {
                            progress_bar
                                .finish_with_message(format!("🔴 {repo_name}  semaphore error"));
                        }
                    }
                    if let Some(coordinator) = coordinator_clone.as_ref() {
                        coordinator.set_status(
                            &repo_name,
                            Status::Error,
                            &format!("semaphore error: {e}"),
                        );
                    }
                    return;
                }
            };
            if let Some(coordinator) = coordinator_clone.as_ref() {
                coordinator.set_stage(&repo_name, Stage::Updating, "git push");
            }

            // Attempt push with retry on rate limit
            let mut attempt = 0;
            let max_attempts = 2;
            let result = loop {
                attempt += 1;
                let (status, message, has_uncommitted) =
                    push_if_needed(&repo_path, &fetch_result, force_push).await;

                // Check for rate limit error
                if message.contains("⚠️ RATE LIMIT") {
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
                        message.replace("⚠️ RATE LIMIT: ", "")
                    );
                    break (status, suggestion, has_uncommitted);
                }

                break (status, message, has_uncommitted);
            };

            let (status, message, has_uncommitted_changes) = result;

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
                    progress_bar
                        .set_message(format!("{:<10}   {}", status.text(), display_message));
                    progress_bar.finish();
                }
            }

            let stats_guard = acquire_stats_lock(&stats_clone);
            stats_guard.update(
                &repo_name,
                &repo_path.to_string_lossy(),
                &status,
                &message,
                has_uncommitted_changes,
            );

            let duration = start_time_clone.elapsed();
            if verbose_clone {
                if let Some(footer_clone) = footer_clone.as_ref() {
                    footer_clone
                        .set_message(stats_guard.generate_summary(total_repos_clone, duration));
                }
            }
            drop(stats_guard);
            if let Some(coordinator) = coordinator_clone.as_ref() {
                coordinator.set_status(&repo_name, status, &message);
            }
        };
        pipeline_futures.push(future);
    }

    while pipeline_futures.next().await.is_some() {}

    if let Some(stop_tx) = hud_stop_tx {
        let _ = stop_tx.send(true);
    }
    if let Some(handle) = hud_handle {
        let _ = handle.await;
    }

    // Show rate limit warning if detected
    if has_rate_limit.load(std::sync::atomic::Ordering::Acquire) {
        let count = rate_limit_count.load(std::sync::atomic::Ordering::Acquire);
        eprintln!("\n⚠️  Rate limit detected on {count} operation(s).");
        eprintln!("💡 Try reducing concurrency: repos push --jobs 3");
    }

    if let Some(footer_pb) = footer_pb {
        footer_pb.finish();
    }

    let final_stats = acquire_stats_lock(&context.statistics);
    let detailed_summary = final_stats.generate_detailed_summary(show_changes);
    if !detailed_summary.is_empty() {
        println!("\n{}", "━".repeat(70));
        println!("{detailed_summary}");
        println!("{}", "━".repeat(70));
    }
    println!();
}

/// Check for subrepo drift and display concise summary
fn check_and_display_drift() {
    // Try to analyze subrepos - if it fails (e.g., no subrepos), silently skip
    if let Ok(statuses) = crate::subrepo::status::analyze_subrepos() {
        // Only display if there's drift to report
        if statuses.iter().any(|s| s.has_drift) {
            crate::subrepo::status::display_drift_summary(&statuses);
        }
    }
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
    use crate::core::config::get_git_concurrency;

    // Set terminal title to indicate repos is running
    set_terminal_title("🔽 repos");

    let (start_time, repos) = init_command(SCANNING_MESSAGE).await;

    if repos.is_empty() {
        println!("\r{NO_REPOS_MESSAGE}");
        // Set terminal title to green checkbox to indicate completion
        set_terminal_title_and_flush("✅ repos");
        return Ok(());
    }

    // Determine concurrency level based on CLI args and system resources
    let concurrent_limit = get_git_concurrency(jobs, sequential);

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    let concurrency_info = if verbose {
        format!(" ({concurrent_limit} concurrent)")
    } else {
        String::new()
    };
    let pull_strategy = if use_rebase { " with rebase" } else { "" };
    if verbose {
        print!(
            "\r🔽 Pulling {total_repos} {repo_word}{pull_strategy}{concurrency_info}                    \n"
        );
        println!();
    }

    // Create processing context with configured concurrency
    let context = match create_processing_context(std::sync::Arc::new(repos), start_time, concurrent_limit) {
        Ok(context) => context,
        Err(e) => {
            // If context creation fails, set completion title and return error
            set_terminal_title_and_flush("✅ repos");
            return Err(e);
        }
    };

    // Process all repositories concurrently
    process_pull_repositories(context, use_rebase, verbose, show_changes).await;

    // Check for subrepo drift unless explicitly skipped
    if !no_drift_check {
        check_and_display_drift();
    }

    // Set terminal title to green checkbox to indicate completion
    set_terminal_title_and_flush("✅ repos");

    Ok(())
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
) {
    use crate::core::{acquire_stats_lock, create_progress_bar};
    use crate::git::{fetch_and_analyze_for_pull, pull_if_needed};
    use futures::stream::{FuturesUnordered, StreamExt};

    let use_hud = !verbose;

    // Use 2x concurrency for fetch phase (I/O bound), standard concurrency for pull phase
    use crate::core::config::FETCH_CONCURRENT_CAP;
    let fetch_concurrency = (context.max_concurrency * 2).min(FETCH_CONCURRENT_CAP);
    let fetch_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(fetch_concurrency));

    let coordinator = if use_hud {
        let repo_names: Vec<String> = context
            .repositories
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        Some(std::sync::Arc::new(SyncCoordinator::new(
            SyncMode::Pull,
            &repo_names,
            context.total_repos,
            fetch_concurrency,
            context.max_concurrency,
            std::sync::Arc::clone(&context.statistics),
        )))
    } else {
        None
    };
    let (hud_stop_tx, hud_handle) = if let Some(coord) = &coordinator {
        let (stop_tx, handle) = coord.start();
        (Some(stop_tx), Some(handle))
    } else {
        (None, None)
    };

    let repo_progress_bars: Vec<Option<indicatif::ProgressBar>> = if verbose {
        context
            .repositories
            .iter()
            .map(|(repo_name, _)| {
                let pb = create_progress_bar(
                    &context.multi_progress,
                    &context.progress_style,
                    repo_name,
                );
                pb.set_message("processing...");
                Some(pb)
            })
            .collect()
    } else {
        vec![None; context.repositories.len()]
    };

    let footer_pb = if verbose {
        let _separator_pb = crate::core::create_separator_progress_bar(&context.multi_progress);
        let footer_pb = crate::core::create_footer_progress_bar(&context.multi_progress);
        footer_pb.set_message(
            "🔽 0 Pulled  🟢 0 Synced  🔴 0 Failed  🟡 0 No Upstream  🟠 0 Skipped".to_string(),
        );
        let _separator_pb2 = crate::core::create_separator_progress_bar(&context.multi_progress);
        Some(footer_pb)
    } else {
        None
    };

    let max_name_length = context.max_name_length;
    let start_time = context.start_time;
    let total_repos = context.total_repos;

    // Track rate limit errors for adaptive backoff
    let rate_limit_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let has_rate_limit = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Track pull statistics
    let total_commits_pulled = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // Create pipelined futures: each does fetch → immediately pull
    let mut pipeline_futures = FuturesUnordered::new();
    for ((repo_name, repo_path), progress_bar) in
        context.repositories.iter().zip(repo_progress_bars)
    {
        let fetch_semaphore_clone = std::sync::Arc::clone(&fetch_semaphore);
        let pull_semaphore_clone = std::sync::Arc::clone(&context.semaphore);
        let stats_clone = std::sync::Arc::clone(&context.statistics);
        let footer_clone = footer_pb.clone();
        let rate_limit_count_clone = std::sync::Arc::clone(&rate_limit_count);
        let has_rate_limit_clone = std::sync::Arc::clone(&has_rate_limit);
        let total_commits_pulled_clone = std::sync::Arc::clone(&total_commits_pulled);
        let coordinator_clone = coordinator.clone();
        let verbose_clone = verbose;
        let max_name_length_clone = max_name_length;
        let start_time_clone = start_time;
        let total_repos_clone = total_repos;

        let future = async move {
            use crate::core::config::SLOW_REPO_THRESHOLD_SECS;

            // Track start time for this repo
            let repo_start_time = std::time::Instant::now();

            // PHASE 1: Fetch with high concurrency
            let _fetch_permit = match fetch_semaphore_clone.acquire().await {
                Ok(permit) => permit,
                Err(e) => {
                    eprintln!(
                        "Error: Failed to acquire fetch permit for {repo_name}: {e}"
                    );

                    // Update statistics to track this failure
                    let stats_guard = acquire_stats_lock(&stats_clone);
                    stats_guard.update(
                        &repo_name,
                        &repo_path.to_string_lossy(),
                        &Status::Error,
                        &format!("semaphore error: {e}"),
                        false,
                    );

                    // Finish progress bar
                    if verbose_clone {
                        if let Some(progress_bar) = progress_bar.as_ref() {
                            progress_bar
                                .finish_with_message(format!("🔴 {repo_name}  semaphore error"));
                        }
                    }
                    if let Some(coordinator) = coordinator_clone.as_ref() {
                        coordinator.set_status(
                            &repo_name,
                            Status::Error,
                            &format!("semaphore error: {e}"),
                        );
                    }
                    return;
                }
            };
            if let Some(coordinator) = coordinator_clone.as_ref() {
                coordinator.set_stage(&repo_name, Stage::Checking, "git fetch --quiet");
            }
            let fetch_result = fetch_and_analyze_for_pull(&repo_path).await;
            drop(_fetch_permit); // Fetch permit released here

            // PHASE 2: Pull with standard concurrency + rate limit protection
            if let Some(coordinator) = coordinator_clone.as_ref() {
                coordinator.set_stage(&repo_name, Stage::Waiting, "waiting on write slot");
            }
            let _pull_permit = match pull_semaphore_clone.acquire().await {
                Ok(permit) => permit,
                Err(e) => {
                    eprintln!(
                        "Error: Failed to acquire pull permit for {repo_name}: {e}"
                    );

                    // Update statistics to track this failure
                    let stats_guard = acquire_stats_lock(&stats_clone);
                    stats_guard.update(
                        &repo_name,
                        &repo_path.to_string_lossy(),
                        &Status::Error,
                        &format!("semaphore error: {e}"),
                        false,
                    );

                    // Finish progress bar
                    if verbose_clone {
                        if let Some(progress_bar) = progress_bar.as_ref() {
                            progress_bar
                                .finish_with_message(format!("🔴 {repo_name}  semaphore error"));
                        }
                    }
                    if let Some(coordinator) = coordinator_clone.as_ref() {
                        coordinator.set_status(
                            &repo_name,
                            Status::Error,
                            &format!("semaphore error: {e}"),
                        );
                    }
                    return;
                }
            };
            if let Some(coordinator) = coordinator_clone.as_ref() {
                let pull_op = if use_rebase {
                    "git pull --rebase --autostash"
                } else {
                    "git pull --ff-only"
                };
                coordinator.set_stage(&repo_name, Stage::Updating, pull_op);
            }

            // Attempt pull with retry on rate limit
            let mut attempt = 0;
            let max_attempts = 2;
            let result = loop {
                attempt += 1;
                let (status, message, has_uncommitted) =
                    pull_if_needed(&repo_path, &fetch_result, use_rebase).await;

                // Check for rate limit error
                if message.contains("⚠️ RATE LIMIT") {
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
                        message.replace("⚠️ RATE LIMIT: ", "")
                    );
                    break (status, suggestion, has_uncommitted);
                }

                break (status, message, has_uncommitted);
            };

            let (status, message, has_uncommitted_changes) = result;

            // Track total commits pulled
            if matches!(status, crate::git::Status::Pulled) {
                total_commits_pulled_clone.fetch_add(
                    fetch_result.behind_count as usize,
                    std::sync::atomic::Ordering::Relaxed,
                );
            }

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
                    progress_bar
                        .set_message(format!("{:<10}   {}", status.text(), display_message));
                    progress_bar.finish();
                }
            }

            let stats_guard = acquire_stats_lock(&stats_clone);
            stats_guard.update(
                &repo_name,
                &repo_path.to_string_lossy(),
                &status,
                &message,
                has_uncommitted_changes,
            );

            let duration = start_time_clone.elapsed();
            if verbose_clone {
                if let Some(footer_clone) = footer_clone.as_ref() {
                    footer_clone
                        .set_message(stats_guard.generate_summary(total_repos_clone, duration));
                }
            }
            drop(stats_guard);
            if let Some(coordinator) = coordinator_clone.as_ref() {
                coordinator.set_status(&repo_name, status, &message);
            }
        };
        pipeline_futures.push(future);
    }

    while pipeline_futures.next().await.is_some() {}

    if let Some(stop_tx) = hud_stop_tx {
        let _ = stop_tx.send(true);
    }
    if let Some(handle) = hud_handle {
        let _ = handle.await;
    }

    // Show rate limit warning if detected
    if has_rate_limit.load(std::sync::atomic::Ordering::Acquire) {
        let count = rate_limit_count.load(std::sync::atomic::Ordering::Acquire);
        eprintln!("\n⚠️  Rate limit detected on {count} operation(s).");
        eprintln!("💡 Try reducing concurrency: repos pull --jobs 3");
    }

    if let Some(footer_pb) = footer_pb {
        footer_pb.finish();
    }

    let final_stats = acquire_stats_lock(&context.statistics);
    let detailed_summary = final_stats.generate_detailed_summary(show_changes);
    if !detailed_summary.is_empty() {
        println!("\n{}", "━".repeat(70));
        println!("{detailed_summary}");
        println!("{}", "━".repeat(70));
    }
    println!();
}
