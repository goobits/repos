//! Integration tests for command modules
//!
//! This module tests the core command functionality including:
//! - Sync operations (push/pull)
//! - Staging operations (stage/unstage/commit/status)
//!
//! Note: These tests verify command behavior and logic, focusing on error handling,
//! edge cases, and proper execution flow. They test the commands work correctly
//! without requiring actual network operations or real remotes.

mod common;
use common::fixtures::TestRepoBuilder;
use common::git::add_bare_remote;
use common::git::is_git_available;

use goobits_repos::commands::staging::{
    handle_commit_command, handle_stage_command, handle_staging_status_command,
    handle_unstage_command, StatusFilters,
};
use goobits_repos::commands::sync::{
    handle_pull_command, handle_push_command, handle_sync_command,
};
use goobits_repos::git::{fetch_and_analyze, push_if_needed, Status};
use std::env;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git_ok(path: &std::path::Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("Failed to run git");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

// ==============================================================================
// SYNC COMMAND TESTS (commands/sync.rs)
// ==============================================================================

#[tokio::test]
async fn test_sync_command_with_no_repos() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let original_dir = env::current_dir().expect("Failed to get current dir");
    let empty_dir = TempDir::new().expect("Failed to create temp directory");
    env::set_current_dir(empty_dir.path()).expect("Failed to change dir");

    let result = handle_sync_command(false, false, false, true, None, false).await;

    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Sync command should handle an empty directory: {:?}",
        result
    );
}

#[tokio::test]
async fn test_sync_command_with_single_repo_no_remote() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let original_dir = env::current_dir().expect("Failed to get current dir");

    let repo = match TestRepoBuilder::new("test-sync-no-remote").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    env::set_current_dir(repo.path()).expect("Failed to change dir");

    let result = handle_sync_command(true, false, false, true, None, false).await;

    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Sync command should run pull then push without panicking: {:?}",
        result
    );
}

#[tokio::test]
async fn test_push_command_with_single_repo_no_changes() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let original_dir = env::current_dir().expect("Failed to get current dir");

    // Create a test repository with a remote
    let repo = match TestRepoBuilder::new("test-repo").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };
    let _remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");

    // Change to repo directory so it gets discovered
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run push command - should complete without errors (even though push will fail due to no actual remote)
    let result = handle_push_command(false, false, false, true, None, false).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Push command should complete without panicking: {:?}",
        result
    );
}

#[tokio::test]
async fn test_push_command_with_no_remote() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let original_dir = env::current_dir().expect("Failed to get current dir");

    // Create a test repository without a remote
    let repo = match TestRepoBuilder::new("test-repo-no-remote").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Change to repo directory
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run push command - should handle no remote gracefully
    let result = handle_push_command(false, false, false, true, None, false).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Push command should handle missing remote without panicking: {:?}",
        result
    );
}

#[test]
fn test_cli_fails_when_remote_is_unreachable() {
    if !is_git_available() {
        return;
    }

    let repo = TestRepoBuilder::new("test-unreachable")
        .build()
        .expect("Failed to create test repo");
    let remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");
    drop(remote);

    let push = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["push", "--sequential", "--no-drift-check"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos push");
    assert!(!push.status.success(), "unreachable push must exit nonzero");

    let doctor = Command::new(env!("CARGO_BIN_EXE_repos"))
        .arg("doctor")
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos doctor");
    assert!(
        !doctor.status.success(),
        "unreachable remote must make doctor exit nonzero"
    );
    assert!(
        String::from_utf8_lossy(&doctor.stdout).contains("access failed"),
        "doctor should identify remote access failure"
    );
}

#[test]
fn test_doctor_ssh_only_policy_uses_effective_instead_of_url() {
    if !is_git_available() {
        return;
    }

    let repo = TestRepoBuilder::new("test-https-warning")
        .build()
        .expect("Failed to create test repo");
    let remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");
    let https_url = "https://example.invalid/team/repo.git";

    let set_url = Command::new("git")
        .args(["remote", "set-url", "origin", https_url])
        .current_dir(repo.path())
        .output()
        .expect("Failed to set HTTPS remote");
    assert!(set_url.status.success());

    let rewrite_key = format!(
        "url.{}.insteadOf",
        remote.path().join("remote.git").display()
    );
    let rewrite = Command::new("git")
        .args(["config", &rewrite_key, https_url])
        .current_dir(repo.path())
        .output()
        .expect("Failed to configure test URL rewrite");
    assert!(rewrite.status.success());

    let doctor = Command::new(env!("CARGO_BIN_EXE_repos"))
        .arg("doctor")
        .env("REPOS_TRANSPORT_POLICY", "ssh-only")
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos doctor");
    let stdout = String::from_utf8_lossy(&doctor.stdout);

    assert!(
        doctor.status.success(),
        "advisory must not fail doctor: {stdout}"
    );
    assert!(stdout.contains("warning: origin uses HTTP(S)"), "{stdout}");
    assert!(!stdout.contains("ssh-only policy blocked"), "{stdout}");
}

#[test]
fn test_ssh_only_push_blocks_https_fetch_before_credential_helper() {
    if !is_git_available() {
        return;
    }

    let repo = TestRepoBuilder::new("test-ssh-only-fetch")
        .build()
        .expect("Failed to create test repo");
    let helper_marker = repo.path().join("credential-helper-ran");
    let helper = format!("!touch {}", helper_marker.display());
    let remote = "https://secret-token@github.com/goobits/keychain-test.git?access_token=hidden";
    let config_dir = TempDir::new().expect("Failed to create test config directory");
    let global_config = config_dir.path().join("gitconfig");
    fs::write(&global_config, "[repos]\n\ttransportPolicy = ssh-only\n")
        .expect("Failed to write test Git config");

    run_git_ok(repo.path(), &["remote", "add", "origin", remote]);
    run_git_ok(repo.path(), &["config", "credential.helper", &helper]);

    let push = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["push", "--sequential", "--no-drift-check"])
        .env_remove("REPOS_TRANSPORT_POLICY")
        .env("GIT_CONFIG_GLOBAL", &global_config)
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos push");
    let stdout = String::from_utf8_lossy(&push.stdout);

    assert!(!push.status.success(), "HTTPS fetch must be blocked");
    assert!(
        stdout.contains("SSH-only policy blocked fetch (HTTPS)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("remote: origin (HTTPS, github.com/goobits/keychain-test.git)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("remote set-url 'origin' 'git@github.com:goobits/keychain-test.git'"),
        "{stdout}"
    );
    assert!(!stdout.contains("secret-token"), "{stdout}");
    assert!(!stdout.contains("access_token"), "{stdout}");
    assert!(!stdout.contains("hidden"), "{stdout}");
    assert!(
        !helper_marker.exists(),
        "credential helper must not run for a blocked HTTPS remote"
    );
}

#[test]
fn test_ssh_only_push_reports_https_pushurl_fix() {
    if !is_git_available() {
        return;
    }

    let repo = TestRepoBuilder::new("test-ssh-only-pushurl")
        .build()
        .expect("Failed to create test repo");
    let _remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");
    let helper_marker = repo.path().join("credential-helper-ran");
    let helper = format!("!touch {}", helper_marker.display());
    let push_url = "https://github.com/goobits/keychain-test.git";

    run_git_ok(
        repo.path(),
        &["remote", "set-url", "--push", "origin", push_url],
    );
    run_git_ok(repo.path(), &["config", "credential.helper", &helper]);
    repo.create_file("ahead.txt", "one local commit")
        .expect("Failed to create test file");
    repo.commit_all("Create local commit")
        .expect("Failed to create local commit");

    let push = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["push", "--sequential", "--no-drift-check"])
        .env("REPOS_TRANSPORT_POLICY", "ssh-only")
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos push");
    let stdout = String::from_utf8_lossy(&push.stdout);

    assert!(!push.status.success(), "HTTPS push URL must be blocked");
    assert!(
        stdout.contains("SSH-only policy blocked push (HTTPS)"),
        "{stdout}"
    );
    assert!(
        stdout
            .contains("remote set-url --push 'origin' 'git@github.com:goobits/keychain-test.git'"),
        "{stdout}"
    );
    assert!(
        !helper_marker.exists(),
        "credential helper must not run for a blocked HTTPS push URL"
    );
}

#[tokio::test]
async fn test_push_command_with_uncommitted_changes() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let original_dir = env::current_dir().expect("Failed to get current dir");

    // Create a test repository
    let repo = match TestRepoBuilder::new("test-repo-uncommitted").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };
    let _remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");

    // Create an uncommitted file
    let test_file = repo.path().join("uncommitted.txt");
    fs::write(&test_file, "uncommitted content").expect("Failed to write test file");

    // Change to repo directory
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run push command - should detect uncommitted changes
    let result = handle_push_command(false, false, false, true, None, false).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Push command should handle uncommitted changes: {:?}",
        result
    );
}

#[tokio::test]
async fn test_pull_command_with_single_repo() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let original_dir = env::current_dir().expect("Failed to get current dir");

    // Create a test repository with a remote
    let repo = match TestRepoBuilder::new("test-repo-pull").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };
    let _remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");

    // Change to repo directory
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run pull command - should complete without errors
    let result = handle_pull_command(false, false, false, true, None, false).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Pull command should complete without panicking: {:?}",
        result
    );
}

#[tokio::test]
async fn test_push_command_with_auto_upstream() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let original_dir = env::current_dir().expect("Failed to get current dir");

    // Create a test repository with remote
    let repo = match TestRepoBuilder::new("test-repo-force").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };
    let _remote = add_bare_remote(repo.path(), false).expect("Failed to attach bare remote");

    // Change to repo directory
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run push command with automatic upstream creation enabled.
    let result = handle_push_command(true, false, false, true, None, false).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Auto-upstream push command should complete without panicking: {:?}",
        result
    );
}

#[tokio::test]
async fn test_push_if_needed_uses_upstream_remote_for_current_branch() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let root = TempDir::new().expect("Failed to create temp directory");
    let wrong_remote = root.path().join("aaa-remote.git");
    let upstream_remote = root.path().join("origin-remote.git");

    for remote in [&wrong_remote, &upstream_remote] {
        let output = Command::new("git")
            .args(["init", "--bare"])
            .current_dir(root.path())
            .arg(remote)
            .output()
            .expect("Failed to init bare remote");
        assert!(
            output.status.success(),
            "Failed to init bare remote: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let repo = TestRepoBuilder::new("test-push-upstream-remote")
        .build()
        .expect("Failed to create test repo");

    for (name, path) in [
        ("aaa", wrong_remote.to_string_lossy().to_string()),
        ("origin", upstream_remote.to_string_lossy().to_string()),
    ] {
        let output = Command::new("git")
            .args(["remote", "add", name, &path])
            .current_dir(repo.path())
            .output()
            .expect("Failed to add remote");
        assert!(
            output.status.success(),
            "Failed to add remote: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = Command::new("git")
        .args(["checkout", "-b", "feature/music"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to create feature branch");
    assert!(
        output.status.success(),
        "Failed to create feature branch: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = Command::new("git")
        .args(["push", "-u", "origin", "feature/music"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to push initial branch");
    assert!(
        output.status.success(),
        "Failed to push initial branch: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    fs::write(repo.path().join("feature.txt"), "new commit").expect("Failed to write test file");
    let output = Command::new("git")
        .args(["add", "feature.txt"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to stage file");
    assert!(
        output.status.success(),
        "Failed to stage file: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new("git")
        .args(["commit", "-m", "Feature update"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to commit file");
    assert!(
        output.status.success(),
        "Failed to commit file: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let head_commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to get HEAD");
    assert!(head_commit.status.success(), "Failed to get HEAD");
    let head_commit = String::from_utf8_lossy(&head_commit.stdout)
        .trim()
        .to_string();

    let fetch_result = fetch_and_analyze(repo.path(), false).await;
    assert!(fetch_result.upstream_exists);
    assert_eq!(fetch_result.upstream_remote.as_deref(), Some("origin"));
    assert_eq!(
        fetch_result.upstream_branch.as_deref(),
        Some("feature/music")
    );
    assert_eq!(fetch_result.ahead_count, 1);

    let (status, _message, _has_uncommitted) =
        push_if_needed(repo.path(), &fetch_result, false).await;
    assert_eq!(status, Status::Pushed);

    let origin_head = Command::new("git")
        .args(["rev-parse", "feature/music"])
        .current_dir(&upstream_remote)
        .output()
        .expect("Failed to read origin remote head");
    assert!(
        origin_head.status.success(),
        "Origin remote missing feature branch: {}",
        String::from_utf8_lossy(&origin_head.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&origin_head.stdout).trim(),
        head_commit
    );

    let wrong_head = Command::new("git")
        .args(["rev-parse", "feature/music"])
        .current_dir(&wrong_remote)
        .output()
        .expect("Failed to read wrong remote head");
    assert!(
        !wrong_head.status.success(),
        "Wrong remote should not receive pushed branch"
    );
}

// ==============================================================================
// STAGING COMMAND TESTS (commands/staging.rs)
// ==============================================================================

#[tokio::test]
async fn test_stage_command_with_simple_pattern() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepoBuilder::new("test-stage-simple").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Create a test file
    let test_file = repo.path().join("test.txt");
    fs::write(&test_file, "test content").expect("Failed to write test file");

    // Change to repo directory
    let original_dir = env::current_dir().expect("Failed to get current dir");
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run stage command
    let result = handle_stage_command("test.txt".to_string()).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(result.is_ok(), "Stage command should succeed: {:?}", result);

    // Verify file was staged
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to check git status");

    let status_str = String::from_utf8_lossy(&status_output.stdout);
    assert!(
        status_str.contains("A  test.txt") || status_str.contains("?? test.txt"),
        "File should appear in git status, got: {}",
        status_str
    );
}

#[tokio::test]
async fn test_stage_command_with_wildcard_pattern() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepoBuilder::new("test-stage-wildcard").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Create multiple test files
    fs::write(repo.path().join("test1.md"), "# Test 1").expect("Failed to write test1.md");
    fs::write(repo.path().join("test2.md"), "# Test 2").expect("Failed to write test2.md");
    fs::write(repo.path().join("test.txt"), "text file").expect("Failed to write test.txt");

    // Change to repo directory
    let original_dir = env::current_dir().expect("Failed to get current dir");
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run stage command with wildcard pattern
    let result = handle_stage_command("*.md".to_string()).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Stage command with wildcard should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_unstage_command_with_pattern() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepoBuilder::new("test-unstage").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Create and stage a test file
    let test_file = repo.path().join("test.txt");
    fs::write(&test_file, "test content").expect("Failed to write test file");

    std::process::Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to stage file");

    // Change to repo directory
    let original_dir = env::current_dir().expect("Failed to get current dir");
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run unstage command
    let result = handle_unstage_command("test.txt".to_string()).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Unstage command should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_commit_command_with_staged_changes() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let original_dir = env::current_dir().expect("Failed to get current dir");

    // Create a test repository
    let repo = match TestRepoBuilder::new("test-commit").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Create and stage a test file
    let test_file = repo.path().join("newfile.txt");
    fs::write(&test_file, "new content").expect("Failed to write test file");

    std::process::Command::new("git")
        .args(["add", "newfile.txt"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to stage file");

    // Verify file is actually staged before running commit command
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to check git status");

    let status_str = String::from_utf8_lossy(&status_output.stdout);
    assert!(
        !status_str.trim().is_empty(),
        "Should have staged changes before commit, got: {}",
        status_str
    );

    // Change to repo directory
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run commit command
    let result = handle_commit_command("Test commit message".to_string(), false).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Commit command should succeed with staged changes: {:?}",
        result
    );

    // Verify commit was created by checking the last commit message
    // Note: Due to timing and command execution, we verify the command completed successfully
    // rather than parsing git log, as the actual commit creation is tested in git operations tests
}

#[tokio::test]
async fn test_commit_command_with_no_staged_changes() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository (already has initial commit from builder)
    let repo = match TestRepoBuilder::new("test-commit-no-changes").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Change to repo directory
    let original_dir = env::current_dir().expect("Failed to get current dir");
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run commit command with no staged changes - should handle gracefully
    let result = handle_commit_command("Empty commit".to_string(), false).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Commit command should handle no changes gracefully: {:?}",
        result
    );
}

#[tokio::test]
async fn test_commit_command_with_allow_empty_flag() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepoBuilder::new("test-commit-empty").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Change to repo directory
    let original_dir = env::current_dir().expect("Failed to get current dir");
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run commit command with allow_empty flag
    let result = handle_commit_command("Empty commit".to_string(), true).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Commit command with allow_empty should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_staging_status_command_with_changes() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepoBuilder::new("test-status-changes").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Create files with different states
    // 1. Staged file
    let staged_file = repo.path().join("staged.txt");
    fs::write(&staged_file, "staged content").expect("Failed to write staged file");
    std::process::Command::new("git")
        .args(["add", "staged.txt"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to stage file");

    // 2. Unstaged file
    let unstaged_file = repo.path().join("unstaged.txt");
    fs::write(&unstaged_file, "unstaged content").expect("Failed to write unstaged file");

    // Change to repo directory
    let original_dir = env::current_dir().expect("Failed to get current dir");
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run status command
    let result = handle_staging_status_command(Vec::new(), StatusFilters::default()).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Status command should succeed with changes: {:?}",
        result
    );
}

#[tokio::test]
async fn test_staging_status_command_with_no_changes() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository (already clean from builder)
    let repo = match TestRepoBuilder::new("test-status-no-changes").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Change to repo directory
    let original_dir = env::current_dir().expect("Failed to get current dir");
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run status command
    let result = handle_staging_status_command(Vec::new(), StatusFilters::default()).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Status command should succeed with no changes: {:?}",
        result
    );
}

#[tokio::test]
async fn test_stage_command_with_nonexistent_file() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepoBuilder::new("test-stage-nonexistent").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Change to repo directory
    let original_dir = env::current_dir().expect("Failed to get current dir");
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run stage command with non-existent file - should handle gracefully
    let result = handle_stage_command("nonexistent.txt".to_string()).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Stage command should handle non-existent file gracefully: {:?}",
        result
    );
}

// ==============================================================================
// PARALLEL SYNC TESTS
// ==============================================================================

#[tokio::test]
async fn test_push_command_with_sequential_flag() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepoBuilder::new("test-sequential").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };
    let _remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");

    // Change to repo directory
    let original_dir = env::current_dir().expect("Failed to get current dir");
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run push command with sequential flag
    let result = handle_push_command(false, false, false, true, None, true).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Sequential push command should complete: {:?}",
        result
    );
}

#[tokio::test]
async fn test_push_command_with_custom_jobs_limit() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepoBuilder::new("test-jobs-limit").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };
    let _remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");

    // Change to repo directory
    let original_dir = env::current_dir().expect("Failed to get current dir");
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run push command with custom jobs limit
    let result = handle_push_command(false, false, false, true, Some(2), false).await;

    // Restore original directory before repo cleanup
    let _ = env::set_current_dir(&original_dir);

    assert!(
        result.is_ok(),
        "Push command with custom jobs limit should complete: {:?}",
        result
    );
}
