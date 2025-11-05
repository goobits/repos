// Tests can use internal paths since we're testing the same crate
use repos::core::SyncStatistics;
use repos::git::UserConfig;

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

// Removed tests for internal validation functions (is_valid_email, is_valid_name)
// These are tested indirectly through validate_user_config() which is part of the public API

#[tokio::test]
async fn test_audit_statistics_creation() {
    use repos::audit::scanner::AuditStatistics;

    let stats = AuditStatistics::new();
    assert_eq!(stats.truffle_stats.total_secrets, 0);
}

#[test]
fn test_staging_status_variants() {
    use repos::git::Status;

    // Test new staging status variants
    assert_eq!(Status::Staged.symbol(), "🟢");
    assert_eq!(Status::Unstaged.symbol(), "🟢");
    assert_eq!(Status::Committed.symbol(), "🟢");
    assert_eq!(Status::NoChanges.symbol(), "🟠");
    assert_eq!(Status::StagingError.symbol(), "🔴");
    assert_eq!(Status::CommitError.symbol(), "🔴");

    assert_eq!(Status::Staged.text(), "staged");
    assert_eq!(Status::Unstaged.text(), "unstaged");
    assert_eq!(Status::Committed.text(), "committed");
    assert_eq!(Status::NoChanges.text(), "no-changes");
    assert_eq!(Status::StagingError.text(), "failed");
    assert_eq!(Status::CommitError.text(), "failed");
}

#[tokio::test]
async fn test_git_staging_operations() {
    use repos::git::{stage_files, unstage_files, has_staged_changes, commit_changes};
    use std::fs;
    use tempfile::TempDir;

    // Create a temporary git repository
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let repo_path = temp_dir.path();

    // Initialize git repo
    let init_result = std::process::Command::new("git")
        .args(["init"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git init");

    if !init_result.status.success() {
        // Skip test if git is not available
        return;
    }

    // Configure git for testing
    std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to set git user name");

    std::process::Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to set git user email");

    // Create a test file
    let test_file = repo_path.join("test.txt");
    fs::write(&test_file, "test content").expect("Failed to write test file");

    // Test has_staged_changes - should be false initially
    let has_changes = has_staged_changes(repo_path).await;
    if let Ok(changes) = has_changes {
        assert!(!changes, "Should have no staged changes initially");
    }

    // Test staging a file
    let stage_result = stage_files(repo_path, "test.txt").await;
    assert!(stage_result.is_ok(), "Staging should succeed");

    // Validate staging actually worked by checking git status
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to check git status");

    let status_str = String::from_utf8_lossy(&status_output.stdout);
    assert!(status_str.contains("A  test.txt") || status_str.contains("M  test.txt"),
        "File should be staged in git status, got: {}", status_str);

    // Test has_staged_changes - should be true after staging
    let has_changes = has_staged_changes(repo_path).await;
    if let Ok(changes) = has_changes {
        assert!(changes, "Should have staged changes after staging");
    }

    // Test committing
    let commit_result = commit_changes(repo_path, "Test commit", false).await;
    if let Ok((success, stdout, _stderr)) = commit_result {
        assert!(success, "Commit should succeed");

        // Validate commit hash is returned
        assert!(!stdout.is_empty(), "Commit should return output with hash");
        assert!(stdout.len() >= 7, "Commit hash should be at least 7 characters");
    }

    // Test has_staged_changes - should be false after commit
    let has_changes = has_staged_changes(repo_path).await;
    if let Ok(changes) = has_changes {
        assert!(!changes, "Should have no staged changes after commit");
    }

    // Test unstaging (stage again first)
    fs::write(&test_file, "modified content").expect("Failed to modify test file");
    let _stage_result = stage_files(repo_path, "test.txt").await;

    let unstage_result = unstage_files(repo_path, "test.txt").await;
    assert!(unstage_result.is_ok(), "Unstaging should succeed");

    // Validate unstaging actually worked
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to check git status after unstaging");

    let status_str = String::from_utf8_lossy(&status_output.stdout);
    assert!(!status_str.contains("M  test.txt"),
        "File should not be staged after unstaging, got: {}", status_str);

    // Test has_staged_changes - should be false after unstaging
    let has_changes = has_staged_changes(repo_path).await;
    if let Ok(changes) = has_changes {
        assert!(!changes, "Should have no staged changes after unstaging");
    }
}

#[tokio::test]
async fn test_staging_with_patterns() {
    use repos::git::{stage_files, unstage_files};
    use std::fs;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let repo_path = temp_dir.path();

    // Initialize git repo
    let init_result = std::process::Command::new("git")
        .args(["init"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git init");

    if !init_result.status.success() {
        return; // Skip if git not available
    }

    // Configure git
    std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to set git user name");

    std::process::Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to set git user email");

    // Create test files
    fs::write(repo_path.join("test1.md"), "# Test 1").expect("Failed to write test1.md");
    fs::write(repo_path.join("test2.md"), "# Test 2").expect("Failed to write test2.md");
    fs::write(repo_path.join("test.txt"), "text file").expect("Failed to write test.txt");

    // Create initial commit so we can test unstaging properly
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(repo_path)
        .output()
        .expect("Failed to stage initial files");

    std::process::Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to create initial commit");

    // Modify files so we can test staging changes
    fs::write(repo_path.join("test1.md"), "# Test 1 Modified").expect("Failed to modify test1.md");
    fs::write(repo_path.join("test2.md"), "# Test 2 Modified").expect("Failed to modify test2.md");
    fs::write(repo_path.join("test.txt"), "text file modified").expect("Failed to modify test.txt");

    // Test staging with wildcard pattern
    let stage_result = stage_files(repo_path, "*.md").await;
    assert!(stage_result.is_ok(), "Staging with pattern should work");

    // Validate pattern staging worked - check both .md files are staged
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to check git status after pattern staging");

    let status_str = String::from_utf8_lossy(&status_output.stdout);
    assert!(status_str.contains("test1.md") && status_str.contains("test2.md"),
        "Both .md files should be staged, got: {}", status_str);
    // Note: test.txt should appear as untracked (??) but not staged
    assert!(!status_str.lines().any(|line| line.starts_with("A  test.txt") || line.starts_with("M  test.txt")),
        "test.txt should not be staged (may appear as untracked), got: {}", status_str);

    // Test unstaging with wildcard pattern
    let unstage_result = unstage_files(repo_path, "*.md").await;
    assert!(unstage_result.is_ok(), "Unstaging with pattern should work");

    // Validate pattern unstaging worked
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to check git status after pattern unstaging");

    let status_str = String::from_utf8_lossy(&status_output.stdout);
    // Check specifically that .md files are not staged (first character is not A or M)
    let staged_md_files: Vec<&str> = status_str.lines()
        .filter(|line| {
            let first_char = line.chars().nth(0).unwrap_or(' ');
            (first_char == 'A' || first_char == 'M') && line.contains(".md")
        })
        .collect();
    assert!(staged_md_files.is_empty(),
        "No .md files should be staged after unstaging, got staged: {:?}", staged_md_files);

    // Test staging all files
    let stage_all_result = stage_files(repo_path, ".").await;
    assert!(stage_all_result.is_ok(), "Staging all files should work");
}

#[test]
fn test_stats_update_with_staging_statuses() {
    use repos::core::SyncStatistics;
    use repos::git::Status;

    let mut stats = SyncStatistics::new();

    // Test staging success statuses
    stats.update("test-repo", "/test/path", &Status::Staged, "staged test.txt", false);
    assert_eq!(stats.synced_repos, 1);

    stats.update("test-repo2", "/test/path2", &Status::Unstaged, "unstaged test.txt", false);
    assert_eq!(stats.synced_repos, 2);

    stats.update("test-repo3", "/test/path3", &Status::Committed, "committed abc1234", false);
    assert_eq!(stats.synced_repos, 3);

    // Test skipped statuses
    stats.update("test-repo4", "/test/path4", &Status::NoChanges, "no changes", false);
    assert_eq!(stats.skipped_repos, 1);

    // Test error statuses
    stats.update("test-repo5", "/test/path5", &Status::StagingError, "staging failed", false);
    assert_eq!(stats.error_repos, 1);

    stats.update("test-repo6", "/test/path6", &Status::CommitError, "commit failed", false);
    assert_eq!(stats.error_repos, 2);
}

#[tokio::test]
async fn test_error_scenarios() {
    use repos::git::{stage_files, unstage_files, commit_changes};
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let repo_path = temp_dir.path();

    // Initialize git repo
    let init_result = std::process::Command::new("git")
        .args(["init"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git init");

    if !init_result.status.success() {
        return; // Skip if git not available
    }

    // Configure git
    std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to set git user name");

    std::process::Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to set git user email");

    // Test staging non-existent file - should fail gracefully
    let stage_result = stage_files(repo_path, "nonexistent.txt").await;
    if let Ok((success, _stdout, stderr)) = stage_result {
        // Git add with non-existent file should fail
        assert!(!success, "Staging non-existent file should fail");
        assert!(!stderr.is_empty(), "Should have error message for non-existent file");
    }

    // Test commit with no staged changes - should fail
    let commit_result = commit_changes(repo_path, "Empty commit", false).await;
    if let Ok((success, _stdout, stderr)) = commit_result {
        assert!(!success, "Commit with no changes should fail");
        // Note: Some git versions return empty stderr for "nothing to commit"
        if !stderr.is_empty() {
            assert!(stderr.contains("nothing to commit") || stderr.contains("no changes added") || stderr.contains("working tree clean"),
                "Should indicate nothing to commit, got: '{}'", stderr);
        }
        // The failure (success = false) itself is the main indicator
    }

    // Test commit with allow_empty flag - should succeed
    let empty_commit_result = commit_changes(repo_path, "Empty commit", true).await;
    if let Ok((success, _stdout, _stderr)) = empty_commit_result {
        assert!(success, "Empty commit with allow_empty should succeed");
    }

    // Test unstaging non-existent file - should handle gracefully
    let unstage_result = unstage_files(repo_path, "nonexistent.txt").await;
    if let Ok((success, _stdout, stderr)) = unstage_result {
        // Unstaging non-existent file might succeed or fail depending on git version
        if !success {
            assert!(!stderr.is_empty(), "Should have error message for invalid unstage");
        }
    }
}

#[test]
fn test_repo_visibility_enum() {
    use repos::git::RepoVisibility;

    // Test enum variants exist
    let public = RepoVisibility::Public;
    let private = RepoVisibility::Private;
    let unknown = RepoVisibility::Unknown;

    // Test equality
    assert_eq!(public, RepoVisibility::Public);
    assert_eq!(private, RepoVisibility::Private);
    assert_eq!(unknown, RepoVisibility::Unknown);

    // Test inequality
    assert_ne!(public, private);
    assert_ne!(public, unknown);
    assert_ne!(private, unknown);

    // Test clone
    let cloned = public.clone();
    assert_eq!(cloned, RepoVisibility::Public);

    // Test debug formatting
    let debug_str = format!("{:?}", public);
    assert!(debug_str.contains("Public"));
}

#[tokio::test]
async fn test_get_repo_visibility_non_github() {
    use repos::git::get_repo_visibility;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let repo_path = temp_dir.path();

    // Initialize git repo
    let init_result = std::process::Command::new("git")
        .args(["init"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git init");

    if !init_result.status.success() {
        return; // Skip if git not available
    }

    // Add a non-GitHub remote (e.g., GitLab)
    std::process::Command::new("git")
        .args(["remote", "add", "origin", "https://gitlab.com/user/repo.git"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to add remote");

    // Should return Unknown for non-GitHub repos
    let visibility = get_repo_visibility(repo_path).await;
    assert_eq!(visibility, repos::git::RepoVisibility::Unknown,
        "Non-GitHub repos should return Unknown visibility");
}

#[tokio::test]
async fn test_get_repo_visibility_no_remote() {
    use repos::git::get_repo_visibility;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let repo_path = temp_dir.path();

    // Initialize git repo without remote
    let init_result = std::process::Command::new("git")
        .args(["init"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git init");

    if !init_result.status.success() {
        return; // Skip if git not available
    }

    // Should return Unknown for repos without remote
    let visibility = get_repo_visibility(repo_path).await;
    assert_eq!(visibility, repos::git::RepoVisibility::Unknown,
        "Repos without remote should return Unknown visibility");
}

#[tokio::test]
async fn test_get_repo_visibility_github_repo() {
    use repos::git::get_repo_visibility;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let repo_path = temp_dir.path();

    // Initialize git repo
    let init_result = std::process::Command::new("git")
        .args(["init"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git init");

    if !init_result.status.success() {
        return; // Skip if git not available
    }

    // Add a GitHub remote
    std::process::Command::new("git")
        .args(["remote", "add", "origin", "https://github.com/user/repo.git"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to add remote");

    // This test will call gh CLI - result depends on whether gh is installed
    let visibility = get_repo_visibility(repo_path).await;

    // Should return Unknown if gh CLI is not available or repo doesn't exist
    // (We can't guarantee a specific result without mocking gh, but we test it doesn't panic)
    assert!(
        matches!(visibility, repos::git::RepoVisibility::Public |
                            repos::git::RepoVisibility::Private |
                            repos::git::RepoVisibility::Unknown),
        "Should return a valid RepoVisibility variant"
    );
}

#[tokio::test]
async fn test_has_uncommitted_changes() {
    use repos::git::has_uncommitted_changes;
    use std::fs;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let repo_path = temp_dir.path();

    // Initialize git repo
    let init_result = std::process::Command::new("git")
        .args(["init"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git init");

    if !init_result.status.success() {
        return; // Skip if git not available
    }

    // Configure git
    std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to set git user name");

    std::process::Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to set git user email");

    // Create and commit a file
    let test_file = repo_path.join("test.txt");
    fs::write(&test_file, "initial content").expect("Failed to write test file");

    std::process::Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to stage file");

    std::process::Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to commit");

    // Should have no uncommitted changes after commit
    let has_changes = has_uncommitted_changes(repo_path).await;
    assert!(!has_changes, "Should have no uncommitted changes after clean commit");

    // Modify the file
    fs::write(&test_file, "modified content").expect("Failed to modify test file");

    // Should detect uncommitted changes
    let has_changes = has_uncommitted_changes(repo_path).await;
    assert!(has_changes, "Should detect uncommitted changes after file modification");

    // Stage the changes
    std::process::Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to stage changes");

    // Should still have uncommitted changes (staged but not committed)
    let has_changes = has_uncommitted_changes(repo_path).await;
    assert!(has_changes, "Should detect staged but uncommitted changes");

    // Commit the changes
    std::process::Command::new("git")
        .args(["commit", "-m", "Second commit"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to commit changes");

    // Should have no uncommitted changes after commit
    let has_changes = has_uncommitted_changes(repo_path).await;
    assert!(!has_changes, "Should have no uncommitted changes after committing staged changes");
}

#[tokio::test]
async fn test_create_and_push_tag() {
    use repos::git::create_and_push_tag;
    use std::fs;
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let repo_path = temp_dir.path();

    // Initialize git repo
    let init_result = std::process::Command::new("git")
        .args(["init"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to run git init");

    if !init_result.status.success() {
        return; // Skip if git not available
    }

    // Configure git
    std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to set git user name");

    std::process::Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to set git user email");

    // Create and commit a file (need at least one commit to tag)
    let test_file = repo_path.join("test.txt");
    fs::write(&test_file, "content").expect("Failed to write test file");

    std::process::Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to stage file");

    std::process::Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to commit");

    // Create a tag (push will fail without remote, but tag creation should work)
    let (success, message) = create_and_push_tag(repo_path, "v1.0.0").await;

    // Tag creation should succeed even if push fails
    assert!(success || message.contains("failed to create tag"),
        "Tag operation should complete (creation succeeds, push may fail): {}", message);

    // Verify tag was created
    let tag_check = std::process::Command::new("git")
        .args(["tag", "-l", "v1.0.0"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to list tags");

    let tags = String::from_utf8_lossy(&tag_check.stdout);
    assert!(tags.contains("v1.0.0"), "Tag v1.0.0 should be created");

    // Try to create the same tag again - should indicate it already exists
    let (success, message) = create_and_push_tag(repo_path, "v1.0.0").await;
    assert!(success, "Should handle existing tag gracefully");
    assert!(message.contains("already exists"),
        "Should indicate tag already exists: {}", message);
}
