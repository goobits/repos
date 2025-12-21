//! Git testing utilities

use anyhow::Result;
use std::path::Path;
use std::process::Command;

/// Sets up a git repository with user config
/// Returns Ok(()) on success, or skips test if git is not available
pub fn setup_git_repo(path: &Path) -> Result<()> {
    // Initialize git repo
    let init_result = Command::new("git")
        .args(["init"])
        .current_dir(path)
        .output()?;

    if !init_result.status.success() {
        anyhow::bail!("Git not available - skipping test");
    }

    // Configure git user
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(path)
        .output()?;

    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(path)
        .output()?;

    // Disable commit signing for tests
    Command::new("git")
        .args(["config", "commit.gpgsign", "false"])
        .current_dir(path)
        .output()?;

    Ok(())
}

/// Creates a test commit in the repository
pub fn create_test_commit(
    path: &Path,
    file_name: &str,
    content: &str,
    message: &str,
) -> Result<()> {
    // Write file
    std::fs::write(path.join(file_name), content)?;

    // Stage file
    Command::new("git")
        .args(["add", file_name])
        .current_dir(path)
        .output()?;

    // Commit
    let commit_result = Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(path)
        .output()?;

    if !commit_result.status.success() {
        anyhow::bail!(
            "Failed to create commit: {}",
            String::from_utf8_lossy(&commit_result.stderr)
        );
    }

    Ok(())
}

/// Creates multiple test repositories in a parent directory
#[allow(dead_code)]
pub fn create_multiple_repos(parent_dir: &Path, count: usize) -> Result<Vec<String>> {
    let mut repo_names = Vec::new();

    for i in 0..count {
        let repo_name = format!("test-repo-{}", i + 1);
        let repo_path = parent_dir.join(&repo_name);
        std::fs::create_dir(&repo_path)?;

        setup_git_repo(&repo_path)?;
        create_test_commit(
            &repo_path,
            "README.md",
            &format!("# Repo {}", i + 1),
            "Initial commit",
        )?;

        repo_names.push(repo_name);
    }

    Ok(repo_names)
}

/// Adds a git remote to a repository
pub fn add_git_remote(path: &Path, remote_name: &str, url: &str) -> Result<()> {
    let result = Command::new("git")
        .args(["remote", "add", remote_name, url])
        .current_dir(path)
        .output()?;

    if !result.status.success() {
        anyhow::bail!(
            "Failed to add remote: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }

    Ok(())
}

/// Checks if git is available in the system
pub fn is_git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}
