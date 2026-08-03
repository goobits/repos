//! Unit tests for SyncStatistics
//! These are in a separate file to keep stats.rs clean

#[cfg(test)]
mod tests {
    use crate::core::{clean_error_message, SyncStatistics};
    use crate::git::Status;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    fn record_reversed_project_pair(
        stats: &SyncStatistics,
        label: &str,
        suffix: &str,
        status: Status,
        message: &str,
        has_uncommitted: bool,
    ) {
        for (name_prefix, project) in [("a", "zeta"), ("z", "alpha")] {
            stats.update(
                &format!("{name_prefix}-{label}"),
                &format!("./{project}/{suffix}"),
                &status,
                message,
                has_uncommitted,
            );
        }
    }

    fn assert_projects_are_grouped(report: &str, suffixes: &[&str]) {
        for suffix in suffixes {
            let alpha = format!("path: ./alpha/{suffix}");
            let zeta = format!("path: ./zeta/{suffix}");
            let alpha_index = report
                .find(&alpha)
                .unwrap_or_else(|| panic!("missing {alpha} in report"));
            let zeta_index = report
                .find(&zeta)
                .unwrap_or_else(|| panic!("missing {zeta} in report"));
            assert!(
                alpha_index < zeta_index,
                "expected {alpha} before {zeta}\n{report}"
            );
        }
    }

    #[test]
    fn test_sync_statistics_initialization() {
        let stats = SyncStatistics::new();
        assert_eq!(stats.synced_repos.load(Ordering::Relaxed), 0);
        assert_eq!(stats.skipped_repos.load(Ordering::Relaxed), 0);
        assert_eq!(stats.error_repos.load(Ordering::Relaxed), 0);
        assert_eq!(stats.uncommitted_count.load(Ordering::Relaxed), 0);
        assert_eq!(stats.total_commits_pushed.load(Ordering::Relaxed), 0);
        assert_eq!(stats.total_commits_pulled.load(Ordering::Relaxed), 0);
        assert_eq!(stats.total_refs_fetched.load(Ordering::Relaxed), 0);
        assert!(stats
            .failed_repos
            .lock()
            .expect("Failed to lock failed_repos mutex in test")
            .is_empty());
        assert!(stats
            .pushed_repo_details
            .lock()
            .expect("Failed to lock pushed_repo_details mutex in test")
            .is_empty());
        assert!(stats
            .pulled_repo_details
            .lock()
            .expect("Failed to lock pulled_repo_details mutex in test")
            .is_empty());
        assert!(stats
            .fetched_repo_details
            .lock()
            .expect("Failed to lock fetched_repo_details mutex in test")
            .is_empty());
        assert!(stats
            .operation_outcomes
            .lock()
            .expect("Failed to lock operation_outcomes mutex in test")
            .is_empty());
    }

    #[test]
    fn test_update_with_pushed_status() {
        let stats = SyncStatistics::new();
        stats.update(
            "repo1",
            "/path/1",
            &Status::Pushed,
            "3 commits pushed",
            false,
        );
        assert_eq!(stats.total_commits_pushed.load(Ordering::Relaxed), 3);
        assert_eq!(stats.synced_repos.load(Ordering::Relaxed), 1); // Pushed also increments synced_repos
        let pushed = stats
            .pushed_repo_details
            .lock()
            .expect("Failed to lock pushed_repo_details mutex in test");
        assert_eq!(
            pushed.as_slice(),
            &[("repo1".to_string(), "/path/1".to_string(), 3)]
        );
    }

    #[test]
    fn test_update_with_synced_status() {
        let stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::Synced, "up to date", false);
        assert_eq!(stats.synced_repos.load(Ordering::Relaxed), 1);
        assert_eq!(stats.total_commits_pushed.load(Ordering::Relaxed), 0);
        assert_eq!(stats.total_commits_pulled.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_update_with_pulled_status() {
        let stats = SyncStatistics::new();
        stats.update(
            "repo1",
            "/path/1",
            &Status::Pulled,
            "7 commits pulled",
            false,
        );
        assert_eq!(stats.pulled_repos.load(Ordering::Relaxed), 1);
        assert_eq!(stats.total_commits_pulled.load(Ordering::Relaxed), 7);
        assert_eq!(stats.synced_repos.load(Ordering::Relaxed), 1);
        let pulled = stats
            .pulled_repo_details
            .lock()
            .expect("Failed to lock pulled_repo_details mutex in test");
        assert_eq!(
            pulled.as_slice(),
            &[("repo1".to_string(), "/path/1".to_string(), 7)]
        );
    }

    #[test]
    fn test_update_with_fetched_status() {
        let stats = SyncStatistics::new();
        stats.update(
            "repo1",
            "/path/1",
            &Status::Fetched,
            "2 remote refs updated",
            false,
        );

        assert_eq!(stats.fetched_repos.load(Ordering::Relaxed), 1);
        assert_eq!(stats.total_refs_fetched.load(Ordering::Relaxed), 2);
        assert_eq!(stats.synced_repos.load(Ordering::Relaxed), 1);
        assert_eq!(
            stats
                .fetched_repo_details
                .lock()
                .expect("Failed to lock fetched_repo_details mutex in test")
                .as_slice(),
            &[("repo1".to_string(), "/path/1".to_string(), 2)]
        );
    }

    #[test]
    fn test_update_with_uncommitted_changes() {
        let stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::Synced, "up to date", true);
        assert_eq!(stats.synced_repos.load(Ordering::Relaxed), 1);
        assert_eq!(stats.uncommitted_count.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_update_with_no_upstream() {
        let stats = SyncStatistics::new();
        stats.update(
            "repo1",
            "/path/1",
            &Status::NoUpstream,
            "no tracking",
            false,
        );
        let outcomes = stats
            .operation_outcomes
            .lock()
            .expect("Failed to lock operation_outcomes mutex in test");
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].repository, "repo1");
        assert_eq!(outcomes[0].status, Status::NoUpstream);
        assert_eq!(stats.skipped_repos.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_update_with_no_remote() {
        let stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::NoRemote, "no remote", false);
        let outcomes = stats
            .operation_outcomes
            .lock()
            .expect("Failed to lock operation_outcomes mutex in test");
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, Status::NoRemote);
        assert_eq!(stats.skipped_repos.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_update_with_error() {
        let stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::Error, "push failed", false);
        assert_eq!(
            stats
                .failed_repos
                .lock()
                .expect("Failed to lock failed_repos mutex in test")
                .len(),
            1
        );
        assert_eq!(stats.error_repos.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_commits_pushed_parsing_single() {
        let stats = SyncStatistics::new();
        stats.update("repo1", "/p1", &Status::Pushed, "1 commit pushed", false);
        assert_eq!(stats.total_commits_pushed.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_commits_pushed_parsing_multiple() {
        let stats = SyncStatistics::new();
        stats.update("repo1", "/p1", &Status::Pushed, "5 commits pushed", false);
        assert_eq!(stats.total_commits_pushed.load(Ordering::Relaxed), 5);

        stats.update("repo2", "/p2", &Status::Pushed, "10 commits pushed", false);
        assert_eq!(stats.total_commits_pushed.load(Ordering::Relaxed), 15);
    }

    #[test]
    fn test_error_message_stored() {
        let stats = SyncStatistics::new();
        stats.update(
            "repo1",
            "/path/1",
            &Status::Error,
            "push failed: permission denied",
            false,
        );

        let failed = stats
            .failed_repos
            .lock()
            .expect("Failed to lock failed_repos mutex in test");
        assert_eq!(failed.len(), 1);
        let (name, path, msg) = &failed[0];
        assert_eq!(name, "repo1");
        assert_eq!(path, "/path/1");
        assert_eq!(msg, "push failed: permission denied");
    }

    #[test]
    fn test_clean_error_message_redacts_http_credentials_and_query() {
        let cleaned = clean_error_message("oops 'https://user:secret@example.com/r.git?t=hidden'");

        assert!(cleaned.contains("https://example.com/r.git"), "{cleaned}");
        assert!(!cleaned.contains("user"));
        assert!(!cleaned.contains("secret"));
        assert!(!cleaned.contains("hidden"));
        assert!(!cleaned.contains('?'));
    }

    #[test]
    fn test_generate_push_live_summary_is_compact_and_colored() {
        let stats = SyncStatistics::new();
        stats.update(
            "clean",
            "/repos/clean",
            &Status::Synced,
            "up to date",
            false,
        );
        stats.update("skipped", "/repos/skipped", &Status::Skip, "skip", false);

        let summary = stats.generate_push_live_summary(5);

        assert!(summary.contains("\x1b["));
        assert!(summary.contains("✓\x1b[0m 1 up to date"));
        assert!(summary.contains("↑\x1b[0m 0 pushed / 0 commits"));
        assert!(summary.contains("·\x1b[0m 1 skipped"));
        assert!(summary.contains("↳ scanning 3 remaining"));
    }

    #[test]
    fn test_generate_pull_summary_mentions_pulled() {
        let stats = SyncStatistics::new();
        stats.synced_repos.store(3, Ordering::Relaxed);
        stats.pulled_repos.store(2, Ordering::Relaxed);
        stats.total_commits_pulled.store(12, Ordering::Relaxed);

        let summary = stats.generate_pull_summary(Duration::from_secs(4));

        assert!(summary.contains("2 pulled (12 commits)"));
        assert!(!summary.contains("pushed"));
    }

    #[test]
    fn test_generate_pull_live_summary_matches_push_shape() {
        let stats = SyncStatistics::new();
        stats.update(
            "updated",
            "/repos/updated",
            &Status::Pulled,
            "4 commits pulled",
            false,
        );
        stats.update("skipped", "/repos/skipped", &Status::Skip, "skip", false);

        let summary = stats.generate_pull_live_summary(4);

        assert!(summary.contains("\x1b["));
        assert!(summary.contains("✓\x1b[0m 0 up to date"));
        assert!(summary.contains("↓\x1b[0m 1 pulled / 4 commits"));
        assert!(summary.contains("·\x1b[0m 1 skipped"));
        assert!(summary.contains("↳ scanning 2 remaining"));
    }

    #[test]
    fn test_live_summary_keeps_skipped_and_follow_up_counts_distinct() {
        let stats = SyncStatistics::new();
        stats.update(
            "blocked",
            "/repos/blocked",
            &Status::NoUpstream,
            "no upstream",
            true,
        );

        let summary = stats.generate_push_live_summary(1);

        assert!(summary.contains("1 skipped"));
        assert!(summary.contains("0 follow-up"));
    }

    #[test]
    fn test_generate_pull_report_lists_pulled_repositories() {
        let stats = SyncStatistics::new();
        stats.update(
            "widgets",
            "/repos/widgets",
            &Status::Pulled,
            "2 commits pulled",
            false,
        );

        let report = stats.generate_pull_report(Duration::from_secs(3), false);

        assert!(report.contains("repos pull"));
        assert!(report.contains("▌ Pulled"));
        assert!(report.contains("widgets"));
        assert!(report.contains("2 commits"));
        assert!(!report.contains("Nothing pulled"));
        assert!(!report.contains("repos push"));
    }

    #[test]
    fn test_generate_pull_report_uses_pull_specific_upstream_action() {
        let stats = SyncStatistics::new();
        stats.update(
            "widgets",
            "/repos/widgets",
            &Status::NoUpstream,
            "no upstream",
            false,
        );

        let report = stats.generate_pull_report(Duration::from_secs(3), false);

        assert!(report.contains("set upstream or skip"));
        assert!(!report.contains("repos push --auto-upstream"));
    }

    #[test]
    fn test_generate_push_report_lists_pushed_repositories() {
        let stats = SyncStatistics::new();
        stats.update(
            "widgets",
            "/repos/widgets",
            &Status::Pushed,
            "2 commits pushed",
            false,
        );

        let report = stats.generate_push_report(Duration::from_secs(3), false);

        assert!(report.contains("Pushed"));
        assert!(report.contains("widgets"));
        assert!(report.contains("2 commits"));
        assert!(report.contains("\x1b["));
    }

    #[test]
    fn test_generate_push_report_pluralizes_and_hides_zero_problem_rows() {
        let stats = SyncStatistics::new();
        stats.update(
            "current",
            "/repos/current",
            &Status::Pushed,
            "1 commit pushed",
            false,
        );

        let report = stats.generate_push_report(Duration::from_secs(3), false);

        assert!(report.contains("1 repo / 1 commit"));
        assert!(report.contains("1 commit"));
        assert!(!report.contains("Failed       0"));
        assert!(!report.contains("Follow-up    0"));
        assert!(!report.contains("Skipped      0"));
    }

    #[test]
    fn test_generate_push_report_names_local_change_repositories() {
        let stats = SyncStatistics::new();
        stats.update("repos", "/workspace", &Status::Synced, "up to date", true);

        let report = stats.generate_push_report(Duration::from_secs(3), false);

        assert!(report.contains("Follow-up    1"));
        assert!(report.contains("repos                    uncommitted changes"));
        assert!(report.contains("does not change outcome counts"));
        assert!(report.contains("repos"));
    }

    #[test]
    fn test_generate_push_report_bullets_multiple_local_change_repositories() {
        let stats = SyncStatistics::new();
        stats.update("repos", "/workspace", &Status::Synced, "up to date", true);
        stats.update(
            "docs",
            "/workspace/docs",
            &Status::Synced,
            "up to date",
            true,
        );

        let report = stats.generate_push_report(Duration::from_secs(3), false);

        assert!(report.contains("Follow-up    2"));
        assert_eq!(report.matches("uncommitted changes").count(), 2);
        assert!(report.contains("repos"));
        assert!(report.contains("docs"));
    }

    #[test]
    fn test_transfer_report_groups_each_section_by_project_path() {
        let stats = SyncStatistics::new();
        record_reversed_project_pair(
            &stats,
            "pushed",
            "apps/pushed",
            Status::Pushed,
            "1 commit pushed",
            false,
        );
        record_reversed_project_pair(
            &stats,
            "failed",
            "packages/failed",
            Status::Error,
            "authentication failed",
            false,
        );
        record_reversed_project_pair(
            &stats,
            "skipped",
            "packages/skipped",
            Status::NoRemote,
            "no remote",
            false,
        );
        record_reversed_project_pair(
            &stats,
            "follow-up",
            "packages/follow-up",
            Status::Synced,
            "up to date",
            true,
        );

        let report = stats.generate_push_report(Duration::ZERO, false);
        assert_projects_are_grouped(
            &report,
            &[
                "apps/pushed",
                "packages/failed",
                "packages/skipped",
                "packages/follow-up",
            ],
        );
    }

    #[test]
    fn test_generate_push_report_dedupes_uncommitted_issue_repositories() {
        let stats = SyncStatistics::new();
        stats.update(
            "doppleganger",
            "/repos/doppleganger",
            &Status::NoUpstream,
            "no upstream",
            true,
        );

        let report = stats.generate_push_report(Duration::from_secs(3), false);
        let occurrences = report.matches("doppleganger").count();

        assert_eq!(occurrences, 2);
        assert!(report.contains("no upstream + uncommitted changes"));
        assert!(!report.contains("repo has uncommitted changes"));
        assert!(!report.contains("Local Changes"));
    }

    #[test]
    fn test_generate_push_report_lists_skipped_repository_with_next_step() {
        let stats = SyncStatistics::new();
        stats.update(
            "assets",
            "/workspace/assets",
            &Status::NoUpstream,
            "no upstream",
            false,
        );

        let report = stats.generate_push_report(Duration::from_secs(3), false);

        assert!(report.contains("▌ Skipped"));
        assert!(report.contains("assets                   no upstream"));
        assert!(report.contains("path: ./assets"));
        assert!(report.contains("next: repos push --auto-upstream"));
        assert!(!report.contains("▌ Needs Work"));
    }

    #[test]
    fn test_generate_push_report_combines_extra_follow_up() {
        let stats = SyncStatistics::new();
        stats.update(
            "assets",
            "/workspace/assets",
            &Status::NoUpstream,
            "no upstream",
            false,
        );
        let extra_lines = vec!["  Nested Drift".to_string(), "  aw 3 copies".to_string()];

        let report = stats.generate_push_report_with_follow_up(
            Duration::from_secs(3),
            false,
            1,
            &extra_lines,
        );

        assert!(report.contains("Skipped      1"));
        assert!(report.contains("Follow-up    1"));
        assert!(report.contains("Nested Drift"));
        assert!(report.contains("aw 3 copies"));
    }

    #[test]
    fn test_generate_push_report_outcomes_are_exclusive_and_failures_are_not_repeated() {
        let stats = SyncStatistics::new();
        stats.update(
            "current",
            "/repos/current",
            &Status::Synced,
            "up to date",
            false,
        );
        stats.update(
            "updated",
            "/repos/updated",
            &Status::Pushed,
            "2 commits pushed",
            false,
        );
        stats.update(
            "blocked",
            "/repos/blocked",
            &Status::Error,
            "authentication failed",
            false,
        );
        stats.update(
            "detached",
            "/repos/detached",
            &Status::Skip,
            "detached HEAD",
            false,
        );

        let report = stats.generate_push_report(Duration::from_secs(3), false);

        assert!(report.contains("Pushed       1 repo / 2 commits"));
        assert!(report.contains("Up to date   1"));
        assert!(report.contains("Failed       1"));
        assert!(report.contains("Skipped      1"));
        assert!(report.contains("Checked      4"));
        assert!(!report.contains("Synced"));
        assert_eq!(report.matches("blocked").count(), 2);
        assert!(!report.contains("blocked                   uncommitted changes"));
    }

    #[test]
    fn test_generate_pull_report_outcomes_are_exclusive_and_attributable() {
        let stats = SyncStatistics::new();
        stats.update(
            "current",
            "/repos/current",
            &Status::Synced,
            "up to date",
            false,
        );
        stats.update(
            "updated",
            "/repos/updated",
            &Status::Pulled,
            "3 commits pulled",
            false,
        );
        stats.update(
            "blocked",
            "/repos/blocked",
            &Status::PullError,
            "authentication failed",
            false,
        );
        stats.update(
            "missing",
            "/repos/missing",
            &Status::NoRemote,
            "no remote",
            false,
        );

        let report = stats.generate_pull_report(Duration::from_secs(3), false);

        assert!(report.contains("Pulled       1 repo / 3 commits"));
        assert!(report.contains("Up to date   1"));
        assert!(report.contains("Failed       1"));
        assert!(report.contains("Skipped      1"));
        assert!(report.contains("Checked      4"));
        assert!(report.contains("▌ Pulled"));
        assert!(report.contains("path: /repos/updated"));
        assert!(report.contains("▌ Failed"));
        assert!(report.contains("path: /repos/blocked"));
        assert!(report.contains("▌ Skipped"));
        assert!(report.contains("path: /repos/missing"));
        assert_eq!(report.matches("blocked").count(), 2);
    }

    #[test]
    fn test_generate_fetch_report_uses_shared_exclusive_contract() {
        let stats = SyncStatistics::new();
        stats.update(
            "current",
            "/repos/current",
            &Status::Synced,
            "up to date",
            false,
        );
        stats.update(
            "updated",
            "/repos/updated",
            &Status::Fetched,
            "2 remote refs updated",
            false,
        );
        stats.update(
            "blocked",
            "/repos/blocked",
            &Status::Error,
            "authentication failed",
            false,
        );
        stats.update(
            "missing",
            "/repos/missing",
            &Status::NoRemote,
            "no remote",
            false,
        );

        let report = stats.generate_fetch_report(Duration::from_secs(3));

        assert!(report.contains("repos fetch"));
        assert!(report.contains("Fetched      1 repo / 2 refs"));
        assert!(report.contains("Up to date   1"));
        assert!(report.contains("Failed       1"));
        assert!(report.contains("Skipped      1"));
        assert!(report.contains("Checked      4"));
        assert!(report.contains("▌ Fetched"));
        assert!(report.contains("path: /repos/updated"));
        assert!(report.contains("▌ Failed"));
        assert!(report.contains("path: /repos/blocked"));
        assert!(report.contains("▌ Skipped"));
        assert!(report.contains("path: /repos/missing"));
    }

    #[test]
    fn test_multiple_updates_accumulate() {
        let stats = SyncStatistics::new();

        stats.update("repo1", "/p1", &Status::Synced, "up to date", false);
        stats.update("repo2", "/p2", &Status::Pushed, "3 commits pushed", false);
        stats.update("repo3", "/p3", &Status::Error, "failed", false);

        assert_eq!(stats.synced_repos.load(Ordering::Relaxed), 2); // Both Synced and Pushed increment synced_repos
        assert_eq!(stats.pushed_repos.load(Ordering::Relaxed), 1);
        assert_eq!(stats.total_commits_pushed.load(Ordering::Relaxed), 3);
        assert_eq!(stats.error_repos.load(Ordering::Relaxed), 1);
    }
}
