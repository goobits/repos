use anyhow::Result;
use goobits_repos::git::{fetch_and_analyze_for_pull, pull_if_needed, Status};
use std::process::Command;
use tempfile::TempDir;

mod common;
use common::git::{setup_git_repo, create_test_commit};

#[tokio::test]
async fn test_pull_merge_conflict_handled() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();

    // 1. Upstream
    let remote_path = root.join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "f.txt", "base", "Init")?;
    
    // 2. Clone to local
    let local_path = root.join("local");
    Command::new("git").args(["clone", remote_path.to_str().unwrap(), local_path.to_str().unwrap()]).output()?;
    setup_git_repo(&local_path)?; // Re-setup to ensure test config (user.name etc)

    // 3. Update remote with a change
    create_test_commit(&remote_path, "f.txt", "remote change", "Remote Update")?;

    // 4. Update local with a CONFLICTING change
    create_test_commit(&local_path, "f.txt", "local conflict", "Local Update")?;

    // 5. Analyze for pull
    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    
    // It should detect diverged
    assert_eq!(fetch_result.status, Status::PullError);
    assert!(fetch_result.message.contains("diverged"));

    // 6. Try pull (should return error immediately because status is PullError)
    let (status, message, _) = pull_if_needed(&local_path, &fetch_result, false).await;
    assert_eq!(status, Status::PullError);
    assert!(message.contains("diverged"));

    Ok(())
}

#[tokio::test]
async fn test_invalid_git_repo_detection() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    
    // Not a git repo
    std::fs::write(repo_path.join("README.md"), "hello")?;

    let fetch_result = fetch_and_analyze_for_pull(repo_path).await;
    assert_eq!(fetch_result.status, Status::NoRemote); // Currently reports NoRemote if git fails

    Ok(())
}

#[tokio::test]
async fn test_corrupt_git_repo_handled() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    setup_git_repo(repo_path)?;
    
    // Corrupt it by removing .git/objects
    let objects_dir = repo_path.join(".git").join("objects");
    if objects_dir.exists() {
        std::fs::remove_dir_all(objects_dir)?;
    }

    let fetch_result = fetch_and_analyze_for_pull(repo_path).await;
    // Should fail gracefully
    assert_eq!(fetch_result.status, Status::NoRemote); // Fallback status

    Ok(())
}