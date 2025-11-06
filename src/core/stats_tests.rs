//! Unit tests for SyncStatistics
//! These are in a separate file to keep stats.rs clean

#[cfg(test)]
mod tests {
    use crate::core::SyncStatistics;
    use crate::git::Status;
    use std::time::Duration;

    #[test]
    fn test_sync_statistics_initialization() {
        let stats = SyncStatistics::new();
        assert_eq!(stats.synced_repos, 0);
        assert_eq!(stats.skipped_repos, 0);
        assert_eq!(stats.error_repos, 0);
        assert_eq!(stats.uncommitted_count, 0);
        assert_eq!(stats.total_commits_pushed, 0);
        assert!(stats.no_upstream_repos.is_empty());
        assert!(stats.no_remote_repos.is_empty());
        assert!(stats.failed_repos.is_empty());
    }

    #[test]
    fn test_update_with_pushed_status() {
        let mut stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::Pushed, "3 commits pushed", false);
        assert_eq!(stats.total_commits_pushed, 3);
        assert_eq!(stats.synced_repos, 1); // Pushed also increments synced_repos
    }

    #[test]
    fn test_update_with_synced_status() {
        let mut stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::Synced, "up to date", false);
        assert_eq!(stats.synced_repos, 1);
        assert_eq!(stats.total_commits_pushed, 0);
    }

    #[test]
    fn test_update_with_uncommitted_changes() {
        let mut stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::Synced, "up to date", true);
        assert_eq!(stats.synced_repos, 1);
        assert_eq!(stats.uncommitted_count, 1);
    }

    #[test]
    fn test_update_with_no_upstream() {
        let mut stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::NoUpstream, "no tracking", false);
        assert_eq!(stats.no_upstream_repos.len(), 1);
        assert_eq!(stats.no_upstream_repos[0].0, "repo1");
    }

    #[test]
    fn test_update_with_no_remote() {
        let mut stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::NoRemote, "no remote", false);
        assert_eq!(stats.no_remote_repos.len(), 1);
        assert_eq!(stats.skipped_repos, 1);
    }

    #[test]
    fn test_update_with_error() {
        let mut stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::Error, "push failed", false);
        assert_eq!(stats.failed_repos.len(), 1);
        assert_eq!(stats.error_repos, 1);
    }

    #[test]
    fn test_commits_pushed_parsing_single() {
        let mut stats = SyncStatistics::new();
        stats.update("repo1", "/p1", &Status::Pushed, "1 commit pushed", false);
        assert_eq!(stats.total_commits_pushed, 1);
    }

    #[test]
    fn test_commits_pushed_parsing_multiple() {
        let mut stats = SyncStatistics::new();
        stats.update("repo1", "/p1", &Status::Pushed, "5 commits pushed", false);
        assert_eq!(stats.total_commits_pushed, 5);

        stats.update("repo2", "/p2", &Status::Pushed, "10 commits pushed", false);
        assert_eq!(stats.total_commits_pushed, 15);
    }

    #[test]
    fn test_error_message_stored() {
        let mut stats = SyncStatistics::new();
        stats.update("repo1", "/path/1", &Status::Error, "push failed: permission denied", false);

        assert_eq!(stats.failed_repos.len(), 1);
        let (name, path, msg) = &stats.failed_repos[0];
        assert_eq!(name, "repo1");
        assert_eq!(path, "/path/1");
        assert_eq!(msg, "push failed: permission denied");
    }

    #[test]
    fn test_generate_summary_not_empty() {
        let mut stats = SyncStatistics::new();
        stats.synced_repos = 5;
        stats.total_commits_pushed = 10;

        let duration = Duration::from_secs(30);
        let summary = stats.generate_summary(10, duration);

        assert!(!summary.is_empty(), "Summary should not be empty");
    }

    #[test]
    fn test_multiple_updates_accumulate() {
        let mut stats = SyncStatistics::new();

        stats.update("repo1", "/p1", &Status::Synced, "up to date", false);
        stats.update("repo2", "/p2", &Status::Pushed, "3 commits pushed", false);
        stats.update("repo3", "/p3", &Status::Error, "failed", false);

        assert_eq!(stats.synced_repos, 2); // Both Synced and Pushed increment synced_repos
        assert_eq!(stats.total_commits_pushed, 3);
        assert_eq!(stats.error_repos, 1);
    }
}
