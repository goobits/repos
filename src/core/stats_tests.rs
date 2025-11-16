//! Unit tests for SyncStatistics
//! These are in a separate file to keep stats.rs clean

#[cfg(test)]
mod tests {
    use crate::core::SyncStatistics;
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
        assert!(stats.no_upstream_repos.lock().expect("Failed to lock no_upstream_repos mutex in test").is_empty());
        assert!(stats.no_remote_repos.lock().expect("Failed to lock no_remote_repos mutex in test").is_empty());
        assert!(stats.failed_repos.lock().expect("Failed to lock failed_repos mutex in test").is_empty());
    }

    #[test]
    fn test_update_with_pushed_status() {
        let stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::Pushed, "3 commits pushed", false);
        assert_eq!(stats.total_commits_pushed.load(Ordering::Relaxed), 3);
        assert_eq!(stats.synced_repos.load(Ordering::Relaxed), 1); // Pushed also increments synced_repos
    }

    #[test]
    fn test_update_with_synced_status() {
        let stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::Synced, "up to date", false);
        assert_eq!(stats.synced_repos.load(Ordering::Relaxed), 1);
        assert_eq!(stats.total_commits_pushed.load(Ordering::Relaxed), 0);
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
        stats.update("repo1", "/path/1", &Status::NoUpstream, "no tracking", false);
        let no_upstream = stats.no_upstream_repos.lock().expect("Failed to lock no_upstream_repos mutex in test");
        assert_eq!(no_upstream.len(), 1);
        assert_eq!(no_upstream[0].0, "repo1");
    }

    #[test]
    fn test_update_with_no_remote() {
        let stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::NoRemote, "no remote", false);
        assert_eq!(stats.no_remote_repos.lock().expect("Failed to lock no_remote_repos mutex in test").len(), 1);
        assert_eq!(stats.skipped_repos.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_update_with_error() {
        let stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::Error, "push failed", false);
        assert_eq!(stats.failed_repos.lock().expect("Failed to lock failed_repos mutex in test").len(), 1);
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
        stats.update("repo1", "/path/1", &Status::Error, "push failed: permission denied", false);

        let failed = stats.failed_repos.lock().expect("Failed to lock failed_repos mutex in test");
        assert_eq!(failed.len(), 1);
        let (name, path, msg) = &failed[0];
        assert_eq!(name, "repo1");
        assert_eq!(path, "/path/1");
        assert_eq!(msg, "push failed: permission denied");
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
    fn test_multiple_updates_accumulate() {
        let stats = SyncStatistics::new();

        stats.update("repo1", "/p1", &Status::Synced, "up to date", false);
        stats.update("repo2", "/p2", &Status::Pushed, "3 commits pushed", false);
        stats.update("repo3", "/p3", &Status::Error, "failed", false);

        assert_eq!(stats.synced_repos.load(Ordering::Relaxed), 2); // Both Synced and Pushed increment synced_repos
        assert_eq!(stats.total_commits_pushed.load(Ordering::Relaxed), 3);
        assert_eq!(stats.error_repos.load(Ordering::Relaxed), 1);
    }
}
