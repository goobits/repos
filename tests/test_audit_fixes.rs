use anyhow::Result;
use goobits_repos::audit::fixes::{apply_fixes, FixOptions};
use goobits_repos::audit::hygiene::{check_repo_hygiene, HygieneStatistics};
use std::process::Command;
use tempfile::TempDir;

mod common;
use common::git::{setup_git_repo, create_test_commit};

#[tokio::test]
async fn test_fix_gitignore_violations() -> Result<()> {
    // 1. Setup
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    setup_git_repo(repo_path)?;

    // Create a violation file (.log files are in UNIVERSAL_BAD_PATTERNS)
    create_test_commit(repo_path, "app.log", "some logs", "Add log file")?;
    
    // 2. Scan
    let (status, message, violations) = check_repo_hygiene(repo_path).await;
    
    // We expect violations
    assert!(violations.iter().any(|v| v.file_path == "app.log"));

    // 3. Construct Stats
    let mut stats = HygieneStatistics::new();
    stats.update("test-repo", repo_path.to_str().unwrap(), &status, &message, violations);

    // 4. Apply Fixes
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

    // 5. Verify
    assert_eq!(results.len(), 1);
    assert!(results[0].errors.is_empty());
    
    // Check .gitignore
    let gitignore_content = std::fs::read_to_string(repo_path.join(".gitignore"))?;
    assert!(gitignore_content.contains("*.log"));

    // Check if file is untracked
    let status_output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_path)
        .output()?;
    let status_text = String::from_utf8(status_output.stdout)?;
    
    // ?? app.log means it's untracked
    // But since it is now in .gitignore, it should NOT show up in status at all!
    // And .gitignore is committed, so it shouldn't show up either.
    // So status should be empty.
    assert!(status_text.trim().is_empty(), "Status should be empty (files ignored), got: {}", status_text);

    Ok(())
}

#[tokio::test]
async fn test_fix_large_files_dry_run() -> Result<()> {
    // Check if git filter-repo is installed
    let filter_repo_check = Command::new("git")
        .args(["filter-repo", "--version"])
        .output();
        
    if filter_repo_check.is_err() || !filter_repo_check.unwrap().status.success() {
        eprintln!("git-filter-repo not installed, skipping large file fix test");
        return Ok(());
    }

    // 1. Setup
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    setup_git_repo(repo_path)?;

    // Create a large file
    // 1MB + 1 byte
    let size = 1_048_577;
    let large_data = vec![0u8; size];
    std::fs::write(repo_path.join("large.bin"), &large_data)?;
    
    Command::new("git")
        .args(["add", "large.bin"])
        .current_dir(repo_path)
        .output()?;
        
    Command::new("git")
        .args(["commit", "-m", "Add large file"])
        .current_dir(repo_path)
        .output()?;

    // 2. Scan
    let (status, message, violations) = check_repo_hygiene(repo_path).await;
    
    // Verify we found it (requires git cat-file to work in scanner)
    assert!(!violations.is_empty());

    // 3. Construct Stats
    let mut stats = HygieneStatistics::new();
    stats.update("test-repo", repo_path.to_str().unwrap(), &status, &message, violations);

    // 4. Apply Fixes (Dry Run)
    let options = FixOptions {
        interactive: false,
        fix_gitignore: false,
        fix_large: true,
        fix_secrets: false,
        untrack_files: false,
        dry_run: true,
        skip_confirm: true,
        target_repos: None,
    };

    let results = apply_fixes(&stats, options).await?;

    // 5. Verify
    assert_eq!(results.len(), 1);
    assert!(!results[0].fixes_applied.is_empty());
    assert!(results[0].fixes_applied[0].contains("[DRY RUN]"));

    Ok(())
}
