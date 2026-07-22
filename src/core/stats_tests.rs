//! Unit tests for SyncStatistics
//! These are in a separate file to keep stats.rs clean

#[cfg(test)]
mod tests {
    use crate::core::{clean_error_message, SyncStatistics};
    use crate::git::Status;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    #[test]
    fn test_sync_statistics_initialization() {
        let stats = SyncStatistics::new();
        assert_eq!(stats.synced_repos.load(Ordering::Relaxed), 0);
        assert_eq!(stats.skipped_repos.load(Ordering::Relaxed), 0);
        assert_eq!(stats.error_repos.load(Ordering::Relaxed), 0);
        assert_eq!(stats.uncommitted_count.load(Ordering::Relaxed), 0);
        assert_eq!(stats.total_commits_pushed.load(Ordering::Relaxed), 0);
        assert_eq!(stats.total_commits_pulled.load(Ordering::Relaxed), 0);
        assert!(stats
            .no_upstream_repos
            .lock()
            .expect("Failed to lock no_upstream_repos mutex in test")
            .is_empty());
        assert!(stats
            .no_remote_repos
            .lock()
            .expect("Failed to lock no_remote_repos mutex in test")
            .is_empty());
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
            .skipped_reasons
            .lock()
            .expect("Failed to lock skipped_reasons mutex in test")
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
        let no_upstream = stats
            .no_upstream_repos
            .lock()
            .expect("Failed to lock no_upstream_repos mutex in test");
        assert_eq!(no_upstream.len(), 1);
        assert_eq!(no_upstream[0].0, "repo1");
    }

    #[test]
    fn test_update_with_no_remote() {
        let stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::NoRemote, "no remote", false);
        assert_eq!(
            stats
                .no_remote_repos
                .lock()
                .expect("Failed to lock no_remote_repos mutex in test")
                .len(),
            1
        );
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
    fn test_generate_summary_not_empty() {
        let stats = SyncStatistics::new();
        stats.synced_repos.store(5, Ordering::Relaxed);
        stats.total_commits_pushed.store(10, Ordering::Relaxed);

        let duration = Duration::from_secs(30);
        let summary = stats.generate_summary(10, duration);

        assert!(!summary.is_empty(), "Summary should not be empty");
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
        assert!(summary.contains("✓\x1b[0m 1 synced"));
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
        assert!(!report.contains("Needs work   0"));
        assert!(!report.contains("Skipped      0"));
    }

    #[test]
    fn test_generate_push_report_names_local_change_repositories() {
        let stats = SyncStatistics::new();
        stats.update("repos", "/workspace", &Status::Synced, "up to date", true);

        let report = stats.generate_push_report(Duration::from_secs(3), false);

        assert!(report.contains("Needs work   1"));
        assert!(report.contains("1 repo has uncommitted changes: repos"));
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

        assert!(report.contains("2 repos have uncommitted changes:"));
        assert!(report.contains("repos"));
        assert!(report.contains("docs"));
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
    fn test_generate_push_report_uses_light_issue_table() {
        let stats = SyncStatistics::new();
        stats.update(
            "assets",
            "/workspace/assets",
            &Status::NoUpstream,
            "no upstream",
            false,
        );

        let report = stats.generate_push_report(Duration::from_secs(3), false);

        assert!(report.contains("Repo                        Reason"));
        assert!(report.contains("──────────────────────────"));
        assert!(report.contains("assets                      no upstream"));
        assert!(report.contains("└─ ./assets"));
        assert!(!report.contains("+--------------------------+"));
    }

    #[test]
    fn test_generate_push_report_combines_extra_needs_work() {
        let stats = SyncStatistics::new();
        stats.update(
            "assets",
            "/workspace/assets",
            &Status::NoUpstream,
            "no upstream",
            false,
        );
        let extra_lines = vec!["  Nested Drift".to_string(), "  aw 3 copies".to_string()];

        let report = stats.generate_push_report_with_needs_work(
            Duration::from_secs(3),
            false,
            1,
            &extra_lines,
        );

        assert!(report.contains("Needs work   2"));
        assert!(report.contains("Nested Drift"));
        assert!(report.contains("aw 3 copies"));
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

    #[test]
    fn test_detailed_summary_keeps_full_actionable_paths() {
        let stats = SyncStatistics::new();
        let long_path = "./packages/deeply/nested/workspace/@goobits/auth";

        stats.update(
            "auth",
            long_path,
            &Status::Error,
            "diverged: 1 ahead, 36 behind",
            false,
        );
        stats.update(
            "upstream",
            "./vendor/resvg/upstream",
            &Status::NoUpstream,
            "no upstream",
            false,
        );
        stats.update(
            "missing",
            "./services/missing-remote",
            &Status::NoRemote,
            "no remote",
            false,
        );

        let detailed = stats.generate_detailed_summary(false);

        assert!(detailed.contains(long_path));
        assert!(detailed.contains("./vendor/resvg/upstream"));
        assert!(detailed.contains("./services/missing-remote"));
        assert!(
            !detailed.contains("./..."),
            "detailed summary paths should be directly usable"
        );
    }
}
