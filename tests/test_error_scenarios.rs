use anyhow::Result;
use goobits_repos::git::{fetch_and_analyze_for_pull, pull_if_needed, Status};
use std::process::Command;
use tempfile::TempDir;

mod common;
use common::git::{create_test_commit, setup_git_repo};

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
    Command::new("git")
        .args([
            "clone",
            remote_path.to_str().unwrap(),
            local_path.to_str().unwrap(),
        ])
        .output()?;
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
async fn test_rebase_conflict_restores_original_checkout() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();
    let remote_path = root.join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "shared.txt", "base", "Init")?;

    let local_path = root.join("local");
    Command::new("git")
        .args([
            "clone",
            remote_path
                .to_str()
                .expect("temporary path should be UTF-8"),
            local_path.to_str().expect("temporary path should be UTF-8"),
        ])
        .output()?;
    setup_git_repo(&local_path)?;

    create_test_commit(&remote_path, "shared.txt", "remote change", "Remote update")?;
    create_test_commit(&local_path, "shared.txt", "local change", "Local update")?;
    let original_head = git_output(&local_path, &["rev-parse", "HEAD"])?;
    let original_branch = git_output(&local_path, &["symbolic-ref", "--short", "HEAD"])?;

    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(fetch_result.status, Status::PullError);

    let (status, message, _) = pull_if_needed(&local_path, &fetch_result, true).await;
    assert_eq!(status, Status::PullError);
    assert!(
        message.contains("aborted and restored original checkout"),
        "{message}"
    );
    assert_eq!(
        git_output(&local_path, &["rev-parse", "HEAD"])?,
        original_head
    );
    assert_eq!(
        git_output(&local_path, &["symbolic-ref", "--short", "HEAD"])?,
        original_branch
    );
    assert!(git_output(&local_path, &["status", "--porcelain"])?.is_empty());

    let rebase_head = Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", "REBASE_HEAD"])
        .current_dir(&local_path)
        .output()?;
    assert!(!rebase_head.status.success());

    Ok(())
}

#[tokio::test]
async fn test_pull_integrates_the_analyzed_upstream_snapshot() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();
    let remote_path = root.join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "base.txt", "base", "Init")?;

    let local_path = root.join("local");
    Command::new("git")
        .args([
            "clone",
            remote_path
                .to_str()
                .expect("temporary path should be UTF-8"),
            local_path.to_str().expect("temporary path should be UTF-8"),
        ])
        .output()?;
    setup_git_repo(&local_path)?;

    create_test_commit(&remote_path, "first.txt", "first", "First remote update")?;
    let analyzed_commit = git_output(&remote_path, &["rev-parse", "HEAD"])?;
    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(fetch_result.status, Status::Synced);
    assert_eq!(fetch_result.behind_count, 1);

    create_test_commit(&remote_path, "second.txt", "second", "Second remote update")?;
    let newer_remote_commit = git_output(&remote_path, &["rev-parse", "HEAD"])?;
    assert_ne!(newer_remote_commit, analyzed_commit);

    let (status, _, _) = pull_if_needed(&local_path, &fetch_result, false).await;
    assert_eq!(status, Status::Pulled);
    assert_eq!(
        git_output(&local_path, &["rev-parse", "HEAD"])?,
        analyzed_commit
    );
    assert!(local_path.join("first.txt").is_file());
    assert!(!local_path.join("second.txt").exists());

    Ok(())
}

fn git_output(path: &std::path::Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).current_dir(path).output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[tokio::test]
async fn test_invalid_git_repo_detection() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();

    // Not a git repo
    std::fs::write(repo_path.join("README.md"), "hello")?;

    let fetch_result = fetch_and_analyze_for_pull(repo_path).await;
    assert_eq!(fetch_result.status, Status::Error);

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
    assert_eq!(fetch_result.status, Status::Error);

    Ok(())
}
