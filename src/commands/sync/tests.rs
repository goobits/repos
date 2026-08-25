use super::format_nested_drift_result;
use super::progress::{spawn_slow_repo_watchdog, stop_slow_repo_watchdog};
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

#[test]
fn incomplete_nested_check_counts_as_follow_up_work() {
    let (follow_up_count, lines) =
        format_nested_drift_result(Err(anyhow::anyhow!("inspection failed")));

    assert_eq!(follow_up_count, 1);
    assert!(lines.join("\n").contains("Drift check incomplete"));
}
