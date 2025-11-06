//! Repository push command implementation
//!
//! This module handles the core push functionality - discovering repositories
//! and pushing any unpushed commits to their upstream remotes.

use anyhow::Result;

use crate::core::{
    create_processing_context, init_command, set_terminal_title, set_terminal_title_and_flush,
    NO_REPOS_MESSAGE,
};

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

    let (start_time, repos) = init_command(SCANNING_MESSAGE);

    if repos.is_empty() {
        println!("\r{}", NO_REPOS_MESSAGE);
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
        format!(" ({} concurrent)", concurrent_limit)
    } else {
        String::new()
    };
    print!(
        "\r🚀 Pushing {} {}{}                    \n",
        total_repos, repo_word, concurrency_info
    );
    println!();

    // Create processing context with configured concurrency
    let context = match create_processing_context(repos, start_time, concurrent_limit) {
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
async fn process_push_repositories(context: crate::core::ProcessingContext, force_push: bool, verbose: bool, show_changes: bool) {
    use crate::core::{acquire_stats_lock, create_progress_bar};
    use crate::git::{fetch_and_analyze, push_if_needed};
    use futures::stream::{FuturesUnordered, StreamExt};

    // Setup progress bars and statistics
    let repo_progress_bars: Vec<_> = if verbose {
        context.repositories.iter()
            .map(|(repo_name, _)| {
                let pb = create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
                pb.set_message("processing...");
                pb
            })
            .collect()
    } else {
        use indicatif::{ProgressBar, ProgressStyle};
        let single_pb = context.multi_progress.add(ProgressBar::new(context.total_repos as u64));
        single_pb.set_style(ProgressStyle::default_bar().template("[{pos}/{len}] {msg}").unwrap());
        single_pb.set_message("📤 Processing...");
        vec![single_pb; context.repositories.len()]
    };

    let _separator_pb = crate::core::create_separator_progress_bar(&context.multi_progress);
    let footer_pb = crate::core::create_footer_progress_bar(&context.multi_progress);
    footer_pb.set_message("✅ 0 Pushed  🟢 0 Synced  🔴 0 Failed  🟡 0 No Upstream  🟠 0 Skipped".to_string());
    let _separator_pb2 = crate::core::create_separator_progress_bar(&context.multi_progress);

    let max_name_length = context.max_name_length;
    let start_time = context.start_time;
    let total_repos = context.total_repos;

    // Use 2x concurrency for fetch phase (I/O bound), standard concurrency for push phase
    use crate::core::config::FETCH_CONCURRENT_CAP;
    let fetch_concurrency = (context.semaphore.available_permits() * 2).min(FETCH_CONCURRENT_CAP);
    let fetch_semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(fetch_concurrency));

    // Track rate limit errors for adaptive backoff
    let rate_limit_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let has_rate_limit = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Create pipelined futures: each does fetch → immediately push
    let mut pipeline_futures = FuturesUnordered::new();
    for ((repo_name, repo_path), progress_bar) in context.repositories.into_iter().zip(repo_progress_bars) {
        let fetch_semaphore_clone = std::sync::Arc::clone(&fetch_semaphore);
        let push_semaphore_clone = std::sync::Arc::clone(&context.semaphore);
        let stats_clone = std::sync::Arc::clone(&context.statistics);
        let footer_clone = footer_pb.clone();
        let rate_limit_count_clone = std::sync::Arc::clone(&rate_limit_count);
        let has_rate_limit_clone = std::sync::Arc::clone(&has_rate_limit);
        let verbose_clone = verbose;
        let max_name_length_clone = max_name_length;
        let start_time_clone = start_time;
        let total_repos_clone = total_repos;

        let future = async move {
            use crate::core::config::SLOW_REPO_THRESHOLD_SECS;

            // Track start time for this repo
            let repo_start_time = std::time::Instant::now();

            // PHASE 1: Fetch with high concurrency
            let fetch_result = {
                let _fetch_permit = fetch_semaphore_clone.acquire().await.expect("Failed to acquire fetch permit");
                fetch_and_analyze(&repo_path, force_push).await
            }; // Fetch permit released here

            // PHASE 2: Push with standard concurrency + rate limit protection
            let _push_permit = push_semaphore_clone.acquire().await.expect("Failed to acquire push permit");

            // Attempt push with retry on rate limit
            let mut attempt = 0;
            let max_attempts = 2;
            let result = loop {
                attempt += 1;
                let (status, message, has_uncommitted) = push_if_needed(&repo_path, fetch_result.clone(), force_push).await;

                // Check for rate limit error
                if message.contains("⚠️ RATE LIMIT") {
                    has_rate_limit_clone.store(true, std::sync::atomic::Ordering::Relaxed);
                    rate_limit_count_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                    if attempt < max_attempts {
                        // Wait briefly and retry
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                        continue;
                    } else {
                        // Max attempts reached, return with suggestion
                        let suggestion = format!(
                            "{} (try reducing concurrency with --jobs N or --sequential)",
                            message.replace("⚠️ RATE LIMIT: ", "")
                        );
                        break (status, suggestion, has_uncommitted);
                    }
                }

                break (status, message, has_uncommitted);
            };

            let (status, message, has_uncommitted_changes) = result;

            // Calculate elapsed time for this repo
            let repo_elapsed = repo_start_time.elapsed();
            let repo_elapsed_secs = repo_elapsed.as_secs_f32();

            let display_message = if has_uncommitted_changes && matches!(status, crate::git::Status::Synced) {
                format!("{} (uncommitted changes)", message)
            } else {
                message.clone()
            };

            // Add elapsed time warning if repo took longer than threshold
            let display_message = if repo_elapsed.as_secs() >= SLOW_REPO_THRESHOLD_SECS {
                format!("{} ({:.1}s)", display_message, repo_elapsed_secs)
            } else {
                display_message
            };

            if verbose_clone {
                progress_bar.set_prefix(format!("{} {:width$}", status.symbol(), repo_name, width = max_name_length_clone));
                progress_bar.set_message(format!("{:<10}   {}", status.text(), display_message));
                progress_bar.finish();
            } else {
                progress_bar.set_message(format!("{} {} ({})", status.symbol(), repo_name, status.text()));
                progress_bar.inc(1);

                // Stream error details immediately for failed repos
                use crate::git::Status;
                if matches!(status, Status::Error) {
                    eprintln!("\n🔴 {} - {}", repo_name, display_message);
                }
            }

            let mut stats_guard = acquire_stats_lock(&stats_clone);
            stats_guard.update(&repo_name, &repo_path.to_string_lossy(), &status, &message, has_uncommitted_changes);

            let duration = start_time_clone.elapsed();
            if verbose_clone {
                footer_clone.set_message(stats_guard.generate_summary(total_repos_clone, duration));
            } else {
                let live_counters = format!(
                    "✅ {} Pushed  🟢 {} Synced  🔴 {} Failed  🟡 {} No Upstream  🟠 {} Skipped",
                    stats_guard.total_commits_pushed, stats_guard.synced_repos, stats_guard.error_repos,
                    stats_guard.no_upstream_repos.len(), stats_guard.skipped_repos
                );
                footer_clone.set_message(live_counters);
            }
        };
        pipeline_futures.push(future);
    }

    while pipeline_futures.next().await.is_some() {}

    // Show rate limit warning if detected
    if has_rate_limit.load(std::sync::atomic::Ordering::Relaxed) {
        let count = rate_limit_count.load(std::sync::atomic::Ordering::Relaxed);
        eprintln!("\n⚠️  Rate limit detected on {} operation(s).", count);
        eprintln!("💡 Try reducing concurrency: repos push --jobs 3");
    }

    footer_pb.finish();

    let final_stats = acquire_stats_lock(&context.statistics);
    let detailed_summary = final_stats.generate_detailed_summary(show_changes);
    if !detailed_summary.is_empty() {
        println!("\n{}", "━".repeat(70));
        println!("{}", detailed_summary);
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
