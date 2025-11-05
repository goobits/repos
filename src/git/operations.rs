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
const GIT_COMMIT_ARGS: &[&str] = &["commit", "-m"];
const GIT_DIFF_CACHED_ARGS: &[&str] = &["diff", "--cached", "--quiet"];

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
pub(crate) async fn get_git_config(path: &Path, key: &str) -> Result<Option<String>> {
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
pub(crate) async fn set_git_config(path: &Path, key: &str, value: &str) -> Result<bool> {
    let args = vec!["config", key, value];

    match run_git(path, &args).await {
        Ok((success, _, _)) => Ok(success),
        Err(e) => Err(e),
    }
}

/// Detects if an error message indicates a rate limit issue
fn is_rate_limit_error(error_msg: &str) -> bool {
    let error_lower = error_msg.to_lowercase();
    error_lower.contains("rate limit")
        || error_lower.contains("too many requests")
        || error_lower.contains("secondary rate limit")
        || (error_lower.contains("403") && error_lower.contains("github"))
}

/// Result of the fetch phase for a repository
#[derive(Clone)]
pub struct FetchResult {
    pub has_uncommitted: bool,
    pub current_branch: String,
    pub ahead_count: u32,
    pub upstream_exists: bool,
    pub status: Status,
    pub message: String,
}

/// Phase 1: Fetch and analyze repository state (read-only, can be highly concurrent)
/// Returns FetchResult with repository state after fetching
pub async fn fetch_and_analyze(path: &Path, force_push: bool) -> FetchResult {
    use crate::core::clean_error_message;

    // Refresh the index to ensure accurate diff-index results
    let _ = run_git(path, &["update-index", "--refresh"]).await;

    // Check if directory has uncommitted changes
    let has_uncommitted = match run_git(path, GIT_DIFF_INDEX_ARGS).await {
        Ok((false, _, _)) => true,
        Ok((true, _, _)) => false,
        Err(_) => false,
    };

    // Get list of remotes
    let remotes = match run_git(path, GIT_REMOTE_ARGS).await {
        Ok((true, output, _)) => output,
        Ok((false, _, _)) | Err(_) => {
            return FetchResult {
                has_uncommitted,
                current_branch: String::new(),
                ahead_count: 0,
                upstream_exists: false,
                status: Status::NoRemote,
                message: STATUS_NO_REMOTE.to_string(),
            };
        }
    };

    if remotes.trim().is_empty() {
        return FetchResult {
            has_uncommitted,
            current_branch: String::new(),
            ahead_count: 0,
            upstream_exists: false,
            status: Status::NoRemote,
            message: STATUS_NO_REMOTE.to_string(),
        };
    }

    // Get current branch
    let current_branch = match run_git(path, GIT_REV_PARSE_HEAD_ARGS).await {
        Ok((true, branch, _)) => branch,
        Ok((false, _, _)) | Err(_) => {
            return FetchResult {
                has_uncommitted,
                current_branch: String::new(),
                ahead_count: 0,
                upstream_exists: false,
                status: Status::Skip,
                message: STATUS_DETACHED_HEAD.to_string(),
            };
        }
    };

    // Skip if in detached HEAD state
    if current_branch == DETACHED_HEAD_BRANCH {
        return FetchResult {
            has_uncommitted,
            current_branch: String::new(),
            ahead_count: 0,
            upstream_exists: false,
            status: Status::Skip,
            message: STATUS_DETACHED_HEAD.to_string(),
        };
    }

    // Fetch latest changes to ensure we have up-to-date refs
    if let Err(e) = run_git(path, GIT_FETCH_ARGS).await {
        let error_message = clean_error_message(&e.to_string());
        let final_message = if is_rate_limit_error(&error_message) {
            format!("⚠️ RATE LIMIT: {}", error_message)
        } else {
            error_message
        };
        return FetchResult {
            has_uncommitted,
            current_branch,
            ahead_count: 0,
            upstream_exists: false,
            status: Status::Error,
            message: final_message,
        };
    }

    // Check if current branch has an upstream
    let upstream_check = run_git(path, &["rev-parse", "--abbrev-ref", "@{upstream}"]).await;
    let upstream_exists = upstream_check.as_ref().is_ok_and(|result| result.0);

    if !upstream_exists {
        let status = if force_push {
            Status::NoUpstream // Will be pushed in phase 2
        } else {
            Status::NoUpstream
        };
        return FetchResult {
            has_uncommitted,
            current_branch,
            ahead_count: 0,
            upstream_exists: false,
            status,
            message: STATUS_NO_UPSTREAM.to_string(),
        };
    }

    // Check if local is ahead of remote
    let ahead_check = run_git(path, &["rev-list", "--count", "HEAD", "^@{upstream}"]).await;
    let ahead_count: u32 = match ahead_check {
        Ok((true, count_str, _)) => count_str.trim().parse().unwrap_or(0),
        _ => 0,
    };

    if ahead_count == 0 {
        FetchResult {
            has_uncommitted,
            current_branch,
            ahead_count: 0,
            upstream_exists: true,
            status: Status::Synced,
            message: STATUS_SYNCED.to_string(),
        }
    } else {
        FetchResult {
            has_uncommitted,
            current_branch,
            ahead_count,
            upstream_exists: true,
            status: Status::Synced, // Will be pushed in phase 2
            message: format!("{} commits ahead", ahead_count),
        }
    }
}

/// Phase 2: Push repository if needed (write operation, moderate concurrency)
/// Returns (status, message, has_uncommitted_changes)
pub async fn push_if_needed(path: &Path, fetch_result: FetchResult, force_push: bool) -> (Status, String, bool) {
    use crate::core::clean_error_message;

    // If already synced or has errors, return immediately
    if fetch_result.status != Status::Synced && fetch_result.status != Status::NoUpstream {
        return (fetch_result.status, fetch_result.message, fetch_result.has_uncommitted);
    }

    // Handle no upstream case
    if !fetch_result.upstream_exists {
        if force_push {
            let push_args = vec!["push", "-u", "origin", &fetch_result.current_branch];
            match run_git(path, &push_args).await {
                Ok((true, _, _)) => {
                    return (
                        Status::Pushed,
                        "set upstream & pushed".to_string(),
                        fetch_result.has_uncommitted,
                    );
                }
                Ok((false, _, stderr)) => {
                    let error_message = clean_error_message(&stderr);
                    return (Status::Error, error_message, fetch_result.has_uncommitted);
                }
                Err(e) => {
                    let error_message = clean_error_message(&e.to_string());
                    return (Status::Error, error_message, fetch_result.has_uncommitted);
                }
            }
        } else {
            return (Status::NoUpstream, STATUS_NO_UPSTREAM.to_string(), fetch_result.has_uncommitted);
        }
    }

    // If no commits ahead, already synced
    if fetch_result.ahead_count == 0 {
        return (Status::Synced, STATUS_SYNCED.to_string(), fetch_result.has_uncommitted);
    }

    // Push changes
    match run_git(path, GIT_PUSH_ARGS).await {
        Ok((true, _, _)) => {
            let commits_word = if fetch_result.ahead_count == 1 {
                "commit"
            } else {
                "commits"
            };
            (
                Status::Pushed,
                format!("{} {} pushed", fetch_result.ahead_count, commits_word),
                fetch_result.has_uncommitted,
            )
        }
        Ok((false, _, stderr)) => {
            let error_message = clean_error_message(&stderr);
            let final_message = if is_rate_limit_error(&error_message) {
                format!("⚠️ RATE LIMIT: {}", error_message)
            } else {
                error_message
            };
            (Status::Error, final_message, fetch_result.has_uncommitted)
        }
        Err(e) => {
            let error_message = clean_error_message(&e.to_string());
            let final_message = if is_rate_limit_error(&error_message) {
                format!("⚠️ RATE LIMIT: {}", error_message)
            } else {
                error_message
            };
            (Status::Error, final_message, fetch_result.has_uncommitted)
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

/// Checks if repository has staged changes ready to commit
/// Returns true if there are staged changes, false if staging area is clean
pub async fn has_staged_changes(path: &Path) -> Result<bool> {
    match run_git(path, GIT_DIFF_CACHED_ARGS).await {
        Ok((success, _, _)) => Ok(!success), // Command succeeds when NO changes (exit 0), so invert
        Err(e) => Err(e),
    }
}

/// Commits staged changes with the given message
/// Returns (success, stdout, stderr)
pub async fn commit_changes(
    path: &Path,
    message: &str,
    allow_empty: bool,
) -> Result<(bool, String, String)> {
    let mut args = Vec::from(GIT_COMMIT_ARGS);
    args.push(message);

    if allow_empty {
        args.insert(1, "--allow-empty"); // Insert after "commit" but before "-m"
    }

    run_git(path, &args).await
}

/// Checks if a repository has uncommitted changes
/// Returns true if there are uncommitted changes, false otherwise
pub async fn has_uncommitted_changes(path: &Path) -> bool {
    // Refresh the index to ensure accurate diff-index results
    let _ = run_git(path, &["update-index", "--refresh"]).await;

    // Check if directory has uncommitted changes
    match run_git(path, GIT_DIFF_INDEX_ARGS).await {
        Ok((false, _, _)) => true, // Command failed means there are changes
        Ok((true, _, _)) => false, // Command succeeded means no changes
        Err(_) => false,           // Error checking, assume no changes
    }
}

/// Creates a git tag and pushes it to the remote
/// Returns (success, message)
pub async fn create_and_push_tag(path: &Path, tag_name: &str) -> (bool, String) {
    // Create the tag
    let tag_result = run_git(path, &["tag", tag_name]).await;

    if let Err(e) = tag_result {
        return (false, format!("failed to create tag: {}", e));
    }

    let (success, _, stderr) = tag_result.unwrap();
    if !success {
        // Tag might already exist
        if stderr.contains("already exists") {
            return (true, "tag already exists".to_string());
        }
        return (false, format!("failed to create tag: {}", stderr));
    }

    // Push the tag
    let push_result = run_git(path, &["push", "origin", tag_name]).await;

    match push_result {
        Ok((true, _, _)) => (true, format!("tagged & pushed {}", tag_name)),
        Ok((false, _, stderr)) => {
            // Tag was created but push failed - that's okay, we'll leave the local tag
            (true, format!("tagged {} (push failed: {})", tag_name, stderr.lines().next().unwrap_or("unknown error")))
        }
        Err(e) => {
            (true, format!("tagged {} (push failed: {})", tag_name, e))
        }
    }
}

/// Repository visibility status
#[derive(Debug, Clone, PartialEq)]
pub enum RepoVisibility {
    Public,
    Private,
    Unknown,
}

/// Detects repository visibility using gh CLI
/// Returns RepoVisibility (defaults to Unknown if gh is not available or repo is not on GitHub)
pub async fn get_repo_visibility(path: &Path) -> RepoVisibility {
    // First check if this is a GitHub repository by looking at the remote URL
    let remote_url = match run_git(path, &["remote", "get-url", "origin"]).await {
        Ok((true, url, _)) => url,
        _ => return RepoVisibility::Unknown,
    };

    // Check if it's a GitHub URL
    if !remote_url.contains("github.com") {
        return RepoVisibility::Unknown;
    }

    // Use gh CLI to check repository visibility
    // gh repo view --json isPrivate returns {"isPrivate": true/false}
    let timeout_duration = Duration::from_secs(10); // Shorter timeout for API calls

    let result = tokio::time::timeout(
        timeout_duration,
        Command::new("gh")
            .args(["repo", "view", "--json", "isPrivate", "-q", ".isPrivate"])
            .current_dir(path)
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            match stdout.as_str() {
                "true" => RepoVisibility::Private,
                "false" => RepoVisibility::Public,
                _ => RepoVisibility::Unknown,
            }
        }
        _ => RepoVisibility::Unknown, // gh CLI not available or command failed
    }
}
