//! Basic git operations and command execution

use anyhow::Result;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

use super::status::Status;

// Timeout constants
const GIT_OPERATION_TIMEOUT_SECS: u64 = 180; // 3 minutes per repository

// Git command arguments
const GIT_DIFF_INDEX_ARGS: &[&str] = &["diff-index", "--quiet", "HEAD", "--"];
const GIT_REMOTE_ARGS: &[&str] = &["remote"];
const GIT_REV_PARSE_HEAD_ARGS: &[&str] = &["rev-parse", "--abbrev-ref", "HEAD"];
const GIT_FETCH_ARGS: &[&str] = &["fetch", "--quiet"];
const GIT_PUSH_ARGS: &[&str] = &["push"];
const GIT_CONFIG_GET_ARGS: &[&str] = &["config", "--get"];
const GIT_ADD_ARGS: &[&str] = &["add"];
const GIT_RESTORE_STAGED_ARGS: &[&str] = &["restore", "--staged"];
const GIT_STATUS_PORCELAIN_ARGS: &[&str] = &["status", "--porcelain"];

// Status messages
const DETACHED_HEAD_BRANCH: &str = "HEAD";
const STATUS_NO_REMOTE: &str = "no remote";
const STATUS_DETACHED_HEAD: &str = "detached HEAD";
const STATUS_NO_UPSTREAM: &str = "no tracking";
const STATUS_SYNCED: &str = "up to date";

/// Runs a git command in the specified directory with a timeout
/// Returns (success, stdout, stderr)
pub async fn run_git(path: &Path, args: &[&str]) -> Result<(bool, String, String)> {
    let timeout_duration = Duration::from_secs(GIT_OPERATION_TIMEOUT_SECS);

    let result = tokio::time::timeout(
        timeout_duration,
        Command::new("git").args(args).current_dir(path).output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => Ok((
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        )),
        Ok(Err(e)) => Err(e.into()),
        Err(_) => Err(anyhow::anyhow!(
            "Git operation timed out after {} seconds",
            GIT_OPERATION_TIMEOUT_SECS
        )),
    }
}

/// Reads a git config value from the specified repository
/// Returns the config value if it exists, None if not found
pub async fn get_git_config(path: &Path, key: &str) -> Result<Option<String>> {
    let mut args = Vec::from(GIT_CONFIG_GET_ARGS);
    args.push(key);

    match run_git(path, &args).await {
        Ok((true, value, _)) => {
            if value.is_empty() {
                Ok(None)
            } else {
                Ok(Some(value))
            }
        }
        Ok((false, _, _)) => Ok(None), // Key not found
        Err(e) => Err(e),
    }
}

/// Sets a git config value in the specified repository (local scope)
/// Returns success status
pub async fn set_git_config(path: &Path, key: &str, value: &str) -> Result<bool> {
    let args = vec!["config", key, value];

    match run_git(path, &args).await {
        Ok((success, _, _)) => Ok(success),
        Err(e) => Err(e),
    }
}

/// Checks a repository for synchronization status and pushes if needed
/// Returns (status, message, has_uncommitted_changes)
pub async fn check_repo(path: &Path, force_push: bool) -> (Status, String, bool) {
    use crate::core::clean_error_message;

    // Check if directory has uncommitted changes
    let has_uncommitted = match run_git(path, GIT_DIFF_INDEX_ARGS).await {
        Ok((false, _, _)) => true, // Command failed means there are changes
        Ok((true, _, _)) => false, // Command succeeded means no changes
        Err(_) => false,           // Error checking, assume no changes
    };

    // Get list of remotes
    let remotes = match run_git(path, GIT_REMOTE_ARGS).await {
        Ok((true, output, _)) => output,
        Ok((false, _, _)) | Err(_) => {
            return (
                Status::NoRemote,
                STATUS_NO_REMOTE.to_string(),
                has_uncommitted,
            );
        }
    };

    if remotes.trim().is_empty() {
        return (
            Status::NoRemote,
            STATUS_NO_REMOTE.to_string(),
            has_uncommitted,
        );
    }

    // Get current branch
    let current_branch = match run_git(path, GIT_REV_PARSE_HEAD_ARGS).await {
        Ok((true, branch, _)) => branch,
        Ok((false, _, _)) | Err(_) => {
            return (
                Status::Skip,
                STATUS_DETACHED_HEAD.to_string(),
                has_uncommitted,
            );
        }
    };

    // Skip if in detached HEAD state
    if current_branch == DETACHED_HEAD_BRANCH {
        return (
            Status::Skip,
            STATUS_DETACHED_HEAD.to_string(),
            has_uncommitted,
        );
    }

    // Fetch latest changes to ensure we have up-to-date refs
    if let Err(e) = run_git(path, GIT_FETCH_ARGS).await {
        let error_message = clean_error_message(&e.to_string());
        return (Status::Error, error_message, has_uncommitted);
    }

    // Check if current branch has an upstream
    let upstream_check = run_git(path, &["rev-parse", "--abbrev-ref", "@{upstream}"]).await;
    let upstream_exists = upstream_check.as_ref().is_ok_and(|result| result.0);

    if !upstream_exists {
        if force_push {
            // Try to set upstream and push
            let push_args = vec!["push", "-u", "origin", &current_branch];
            match run_git(path, &push_args).await {
                Ok((true, _, _)) => {
                    return (
                        Status::Pushed,
                        "set upstream & pushed".to_string(),
                        has_uncommitted,
                    );
                }
                Ok((false, _, stderr)) => {
                    let error_message = clean_error_message(&stderr);
                    return (Status::Error, error_message, has_uncommitted);
                }
                Err(e) => {
                    let error_message = clean_error_message(&e.to_string());
                    return (Status::Error, error_message, has_uncommitted);
                }
            }
        } else {
            return (
                Status::NoUpstream,
                STATUS_NO_UPSTREAM.to_string(),
                has_uncommitted,
            );
        }
    }

    // Check if local is ahead of remote
    let ahead_check = run_git(path, &["rev-list", "--count", "HEAD", "^@{upstream}"]).await;
    let ahead_count: u32 = match ahead_check {
        Ok((true, count_str, _)) => count_str.trim().parse().unwrap_or(0),
        _ => 0,
    };

    if ahead_count == 0 {
        return (Status::Synced, STATUS_SYNCED.to_string(), has_uncommitted);
    }

    // Push changes
    match run_git(path, GIT_PUSH_ARGS).await {
        Ok((true, _, _)) => {
            let commits_word = if ahead_count == 1 {
                "commit"
            } else {
                "commits"
            };
            (
                Status::Pushed,
                format!("{} {} pushed", ahead_count, commits_word),
                has_uncommitted,
            )
        }
        Ok((false, _, stderr)) => {
            let error_message = clean_error_message(&stderr);
            (Status::Error, error_message, has_uncommitted)
        }
        Err(e) => {
            let error_message = clean_error_message(&e.to_string());
            (Status::Error, error_message, has_uncommitted)
        }
    }
}

/// Stages files matching the given pattern in the specified repository
/// Returns (success, stdout, stderr)
pub async fn stage_files(path: &Path, pattern: &str) -> Result<(bool, String, String)> {
    let mut args = Vec::from(GIT_ADD_ARGS);
    args.push(pattern);
    run_git(path, &args).await
}

/// Unstages files matching the given pattern in the specified repository
/// Returns (success, stdout, stderr)
pub async fn unstage_files(path: &Path, pattern: &str) -> Result<(bool, String, String)> {
    let mut args = Vec::from(GIT_RESTORE_STAGED_ARGS);
    args.push(pattern);
    run_git(path, &args).await
}

/// Gets the staging status of the repository
/// Returns (stdout, stderr) with git status --porcelain output
pub async fn get_staging_status(path: &Path) -> Result<(String, String)> {
    match run_git(path, GIT_STATUS_PORCELAIN_ARGS).await {
        Ok((_, stdout, stderr)) => Ok((stdout, stderr)),
        Err(e) => Err(e),
    }
}
