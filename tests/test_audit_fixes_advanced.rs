use anyhow::Result;
use goobits_repos::audit::fixes::{apply_fixes, FixOptions};
use goobits_repos::audit::hygiene::{check_repo_hygiene, HygieneStatistics};
use std::process::Command;
use tempfile::TempDir;

mod common;
use common::git::{create_test_commit, setup_git_repo};

fn is_tool_installed(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn test_fix_large_actual_removal() -> Result<()> {
    if !is_tool_installed("git-filter-repo") {
        eprintln!("git-filter-repo not installed, skipping test");
        return Ok(());
    }

    // 1. Setup repo with a large file in history
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    setup_git_repo(repo_path)?;

    let large_file = "huge.dat";
    let size = 1_048_577; // > 1MB
    std::fs::write(repo_path.join(large_file), vec![0u8; size])?;

    Command::new("git")
        .args(["add", large_file])
        .current_dir(repo_path)
        .output()?;
    Command::new("git")
        .args(["commit", "-m", "Add large file"])
        .current_dir(repo_path)
        .output()?;

    // 2. Scan
    let (status, message, violations) = check_repo_hygiene(repo_path).await;
    assert!(!violations.is_empty());

    let mut stats = HygieneStatistics::new();
    stats.update(
        "test-repo",
        repo_path.to_str().unwrap(),
        &status,
        &message,
        violations,
    );

    // 3. Fix
    let options = FixOptions {
        interactive: false,
        fix_gitignore: false,
        fix_large: true,
        fix_secrets: false,
        untrack_files: false,
        dry_run: false,
        skip_confirm: true,
        target_repos: None,
    };

    apply_fixes(&stats, options).await?;

    // 4. Verify gone from history
    let output = Command::new("git")
        .args(["rev-list", "--objects", "--all"])
        .current_dir(repo_path)
        .output()?;
    let history = String::from_utf8_lossy(&output.stdout);
    assert!(
        !history.contains(large_file),
        "File should be removed from history"
    );

    Ok(())
}

#[tokio::test]
async fn test_rollback_from_backup_ref() -> Result<()> {
    if !is_tool_installed("git-filter-repo") {
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    setup_git_repo(repo_path)?;

    let large_file = "huge.dat";
    std::fs::write(repo_path.join(large_file), vec![0u8; 1_048_577])?;
    Command::new("git")
        .args(["add", large_file])
        .current_dir(repo_path)
        .output()?;
    Command::new("git")
        .args(["commit", "-m", "Add large file"])
        .current_dir(repo_path)
        .output()?;
    let original_head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()?;
    let original_head_hash = String::from_utf8(original_head.stdout)?.trim().to_string();

    let (status, message, violations) = check_repo_hygiene(repo_path).await;
    let mut stats = HygieneStatistics::new();
    stats.update(
        "test-repo",
        repo_path.to_str().unwrap(),
        &status,
        &message,
        violations,
    );

    let options = FixOptions {
        interactive: false,
        fix_gitignore: false,
        fix_large: true,
        fix_secrets: false,
        untrack_files: false,
        dry_run: false,
        skip_confirm: true,
        target_repos: None,
    };

    let results = apply_fixes(&stats, options).await?;

    // Extract backup ref from message
    let fix_msg = &results[0].fixes_applied[0];
    // "Removed 1 large files from history\n    Recovery: git reset --hard refs/original/pre-fix-backup-large-..."
    let backup_ref = fix_msg
        .split("Recovery: git reset --hard ")
        .nth(1)
        .unwrap()
        .trim();

    // Reset to backup
    Command::new("git")
        .args(["reset", "--hard", backup_ref])
        .current_dir(repo_path)
        .output()?;

    let new_head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()?;
    let new_head_hash = String::from_utf8(new_head.stdout)?.trim().to_string();

    assert_eq!(
        original_head_hash, new_head_hash,
        "Should have rolled back to original HEAD"
    );
    assert!(
        repo_path.join(large_file).exists(),
        "Large file should be back after rollback"
    );

    Ok(())
}

#[tokio::test]
async fn test_fix_concurrent_operations() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();

    let mut stats = HygieneStatistics::new();

    for i in 1..=3 {
        let repo_name = format!("repo-{}", i);
        let repo_path = root.join(&repo_name);
        std::fs::create_dir(&repo_path)?;
        setup_git_repo(&repo_path)?;

        create_test_commit(&repo_path, &format!("app-{}.log", i), "logs", "Add log")?;

        let (status, message, violations) = check_repo_hygiene(&repo_path).await;
        stats.update(
            &repo_name,
            repo_path.to_str().unwrap(),
            &status,
            &message,
            violations,
        );
    }

    let options = FixOptions {
        interactive: false,
        fix_gitignore: true,
        fix_large: false,
        fix_secrets: false,
        untrack_files: true,
        dry_run: false,
        skip_confirm: true,
        target_repos: None,
    };

    let results = apply_fixes(&stats, options).await?;
    assert_eq!(results.len(), 3);
    for r in results {
        assert!(r.errors.is_empty());
    }

    Ok(())
}
