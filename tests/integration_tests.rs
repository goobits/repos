use repos::core::stats::SyncStatistics;
use repos::git::config::{is_valid_email, is_valid_name, UserConfig};

#[test]
fn test_sync_stats_initialization() {
    let stats = SyncStatistics::new();
    assert_eq!(stats.synced_repos, 0);
    assert_eq!(stats.skipped_repos, 0);
    assert_eq!(stats.error_repos, 0);
    assert_eq!(stats.uncommitted_count, 0);
}

#[test]
fn test_user_config_creation() {
    let config = UserConfig::new(
        Some("Test User".to_string()),
        Some("test@example.com".to_string()),
    );
    assert!(!config.is_empty());

    let empty_config = UserConfig::new(None, None);
    assert!(empty_config.is_empty());
}

#[test]
fn test_git_config_validation() {
    // Test invalid email
    assert!(!is_valid_email(""));
    assert!(!is_valid_email("invalid"));
    assert!(!is_valid_email("@domain.com"));

    // Test valid email
    assert!(is_valid_email("user@example.com"));
    assert!(is_valid_email("test.user+tag@domain.co.uk"));
}

#[test]
fn test_git_config_name_validation() {
    // Test invalid names
    assert!(!is_valid_name(""));
    assert!(!is_valid_name("   "));

    // Test valid names
    assert!(is_valid_name("John Doe"));
    assert!(is_valid_name("Alice"));
    assert!(is_valid_name("Bob Smith-Jones"));
}

#[tokio::test]
async fn test_audit_statistics_creation() {
    use repos::audit::scanner::AuditStatistics;

    let stats = AuditStatistics::new();
    assert_eq!(stats.truffle_stats.total_secrets, 0);
}
