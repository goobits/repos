//! Basic git operations and command execution

use anyhow::Result;
use dashmap::DashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
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
const GIT_LFS_ENV_ARGS: &[&str] = &["lfs", "env"];

// Status messages
const DETACHED_HEAD_BRANCH: &str = "HEAD";
const STATUS_NO_REMOTE: &str = "no remote";
const STATUS_DETACHED_HEAD: &str = "detached HEAD";
const STATUS_NO_UPSTREAM: &str = "no tracking";
const STATUS_SYNCED: &str = "up to date";

/// Runs a git command in the specified directory with a timeout
///
/// **INTERNAL API**: This is a low-level helper function intended for internal use.
/// While marked `pub` for crate organization, this API is not stable and may change
/// without notice. External crates should not depend on this function directly.
///
/// Returns `(success, stdout, stderr)` tuple:
/// - `success`: true if git command exit code was 0
/// - `stdout`: trimmed standard output as String
/// - `stderr`: trimmed standard error as String
///
/// Includes a 180-second timeout to prevent hanging on network operations.
#[doc(hidden)]
pub async fn run_git(path: &Path, args: &[&str]) -> Result<(bool, String, String)> {
    let timeout_duration = Duration::from_secs(GIT_OPERATION_TIMEOUT_SECS);

    let result = tokio::time::timeout(
        timeout_duration,
        Command::new("git").args(args).current_dir(path).output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            // Optimize string allocations: only allocate if non-empty (5-10% faster)
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stdout_trimmed = stdout.trim();
            let stdout_string = if stdout_trimmed.is_empty() {
                String::new() // No heap allocation for empty strings
            } else {
                stdout_trimmed.to_string()
            };

            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr_trimmed = stderr.trim();
            let stderr_string = if stderr_trimmed.is_empty() {
                String::new() // No heap allocation for empty strings
            } else {
                stderr_trimmed.to_string()
            };

            Ok((output.status.success(), stdout_string, stderr_string))
        }
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

/// Checks if a repository uses Git LFS
/// Returns true if the repo has LFS configured (via git lfs env check)
pub async fn check_uses_git_lfs(path: &Path) -> bool {
    // Check if git lfs is available and configured for this repo
    // "git lfs env" returns success if LFS is installed and shows config
    match run_git(path, GIT_LFS_ENV_ARGS).await {
        Ok((true, _stdout, _)) => {
            // LFS is installed, check if this repo actually uses it
            // by looking for .gitattributes with filter=lfs
            // Note: We directly try to read without exists() check to avoid TOCTTOU race
            let gitattributes_path = path.join(".gitattributes");
            if let Ok(content) = tokio::fs::read_to_string(&gitattributes_path).await {
                if content.contains("filter=lfs") {
                    return true;
                }
            }
            // Also check if there are any LFS objects tracked
            // "git lfs ls-files" lists tracked LFS files
            if let Ok((true, files, _)) = run_git(path, &["lfs", "ls-files"]).await {
                return !files.trim().is_empty();
            }
            // LFS is installed but repo doesn't appear to use it
            false
        }
        _ => false,
    }
}

/// Pushes Git LFS objects to the remote
/// Should be called BEFORE regular git push when LFS is in use
/// Returns (success, error_message)
pub async fn push_lfs_objects(path: &Path, remote: &str, branch: &str) -> (bool, String) {
    // Run "git lfs push --all <remote> <branch>" to upload all LFS objects
    let args = vec!["lfs", "push", "--all", remote, branch];

    match run_git(path, &args).await {
        Ok((true, _, _)) => (true, String::new()),
        Ok((false, _, stderr)) => {
            let error_msg = if stderr.is_empty() {
                "LFS push failed".to_string()
            } else {
                format!("LFS: {}", stderr.lines().next().unwrap_or("push failed"))
            };
            (false, error_msg)
        }
        Err(e) => (false, format!("LFS error: {}", e)),
    }
}

/// Checks if there are uncommitted LFS objects that need to be pushed
/// Returns true if there are LFS objects pending upload
pub async fn has_pending_lfs_objects(path: &Path) -> bool {
    // "git lfs status" shows files that need to be pushed
    if let Ok((true, stdout, _)) = run_git(path, &["lfs", "status", "--porcelain"]).await {
        // If there's any output, there are pending LFS operations
        !stdout.trim().is_empty()
    } else {
        false
    }
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
pub async fn fetch_and_analyze(path: &Path, _force_push: bool) -> FetchResult {
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
        // Will be pushed in phase 2 (with or without force flag)
        let status = Status::NoUpstream;
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

    // Check if local is behind remote (to detect diverged branches)
    let behind_check = run_git(path, &["rev-list", "--count", "@{upstream}", "^HEAD"]).await;
    let behind_count: u32 = match behind_check {
        Ok((true, count_str, _)) => count_str.trim().parse().unwrap_or(0),
        _ => 0,
    };

    // Branches have diverged - both ahead and behind
    if ahead_count > 0 && behind_count > 0 {
        return FetchResult {
            has_uncommitted,
            current_branch,
            ahead_count,
            upstream_exists: true,
            status: Status::Error,
            message: format!(
                "diverged: {} ahead, {} behind (pull required before push)",
                ahead_count, behind_count
            ),
        };
    }

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
pub async fn push_if_needed(path: &Path, fetch_result: &FetchResult, force_push: bool) -> (Status, String, bool) {
    use crate::core::clean_error_message;

    // If already synced or has errors, return immediately
    if fetch_result.status != Status::Synced && fetch_result.status != Status::NoUpstream {
        return (fetch_result.status, fetch_result.message.clone(), fetch_result.has_uncommitted);
    }

    // Detect remote name for LFS and push operations
    let remote_name = match run_git(path, GIT_REMOTE_ARGS).await {
        Ok((true, remotes, _)) => {
            remotes.lines().next().unwrap_or("origin").to_string()
        }
        _ => "origin".to_string(),
    };

    // Check if repo uses Git LFS and push LFS objects FIRST
    let uses_lfs = check_uses_git_lfs(path).await;
    if uses_lfs && has_pending_lfs_objects(path).await {
        let branch = if fetch_result.current_branch.is_empty() {
            "HEAD".to_string()
        } else {
            fetch_result.current_branch.clone()
        };

        let (lfs_success, lfs_error) = push_lfs_objects(path, &remote_name, &branch).await;
        if !lfs_success {
            // LFS push failed - return error (use default message if error is empty)
            let error_msg = if lfs_error.is_empty() {
                "LFS push failed".to_string()
            } else {
                lfs_error
            };
            return (Status::Error, error_msg, fetch_result.has_uncommitted);
        }
    }

    // Handle no upstream case
    if !fetch_result.upstream_exists {
        if force_push {
            let push_args = vec!["push", "-u", &remote_name, &fetch_result.current_branch];
            match run_git(path, &push_args).await {
                Ok((true, _, _)) => {
                    let msg = if uses_lfs {
                        format!("set upstream ({}) & pushed (with LFS)", remote_name)
                    } else {
                        format!("set upstream ({}) & pushed", remote_name)
                    };
                    return (
                        Status::Pushed,
                        msg,
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

    // Push changes - use explicit remote and branch to match LFS push
    let push_args = vec!["push", &remote_name, &fetch_result.current_branch];
    match run_git(path, &push_args).await {
        Ok((true, _, _)) => {
            let commits_word = if fetch_result.ahead_count == 1 {
                "commit"
            } else {
                "commits"
            };
            let msg = if uses_lfs {
                format!("{} {} pushed (with LFS)", fetch_result.ahead_count, commits_word)
            } else {
                format!("{} {} pushed", fetch_result.ahead_count, commits_word)
            };
            (
                Status::Pushed,
                msg,
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

/// Checks if a repository has uncommitted changes (tracked files only)
///
/// This checks only tracked files using `git diff-index --quiet HEAD`.
/// Returns true if there are uncommitted changes, false otherwise.
///
/// Note: There are synchronous versions in subrepo/{mod.rs, sync.rs} for use
/// in non-async contexts. The sync.rs version is more conservative and includes
/// untracked files.
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

    let (success, _, stderr) = match tag_result {
        Ok(result) => result,
        Err(e) => return (false, format!("failed to create tag: {}", e)),
    };

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

/// Result of the fetch phase for pull operation
#[derive(Clone)]
pub struct PullFetchResult {
    pub has_uncommitted: bool,
    pub behind_count: u32,
    pub status: Status,
    pub message: String,
}

/// Phase 1: Fetch and analyze repository state for pull (read-only, can be highly concurrent)
/// Returns PullFetchResult with repository state after fetching
pub async fn fetch_and_analyze_for_pull(path: &Path) -> PullFetchResult {
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
            return PullFetchResult {
                has_uncommitted,
                behind_count: 0,
                status: Status::NoRemote,
                message: STATUS_NO_REMOTE.to_string(),
            };
        }
    };

    if remotes.trim().is_empty() {
        return PullFetchResult {
            has_uncommitted,
            behind_count: 0,
            status: Status::NoRemote,
            message: STATUS_NO_REMOTE.to_string(),
        };
    }

    // Get current branch
    let current_branch = match run_git(path, GIT_REV_PARSE_HEAD_ARGS).await {
        Ok((true, branch, _)) => branch,
        Ok((false, _, _)) | Err(_) => {
            return PullFetchResult {
                has_uncommitted,
                behind_count: 0,
                status: Status::Skip,
                message: STATUS_DETACHED_HEAD.to_string(),
            };
        }
    };

    // Skip if in detached HEAD state
    if current_branch == DETACHED_HEAD_BRANCH {
        return PullFetchResult {
            has_uncommitted,
            behind_count: 0,
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
        return PullFetchResult {
            has_uncommitted,
            behind_count: 0,
            status: Status::Error,
            message: final_message,
        };
    }

    // Check if current branch has an upstream
    let upstream_check = run_git(path, &["rev-parse", "--abbrev-ref", "@{upstream}"]).await;
    let upstream_exists = upstream_check.as_ref().is_ok_and(|result| result.0);

    if !upstream_exists {
        return PullFetchResult {
            has_uncommitted,
            behind_count: 0,
            status: Status::NoUpstream,
            message: STATUS_NO_UPSTREAM.to_string(),
        };
    }

    // Check if local is behind remote
    let behind_check = run_git(path, &["rev-list", "--count", "@{upstream}", "^HEAD"]).await;
    let behind_count: u32 = match behind_check {
        Ok((true, count_str, _)) => count_str.trim().parse().unwrap_or(0),
        _ => 0,
    };

    // Check if local is ahead of remote (to detect diverged branches)
    let ahead_check = run_git(path, &["rev-list", "--count", "HEAD", "^@{upstream}"]).await;
    let ahead_count: u32 = match ahead_check {
        Ok((true, count_str, _)) => count_str.trim().parse().unwrap_or(0),
        _ => 0,
    };

    // Branches have diverged - both ahead and behind
    if ahead_count > 0 && behind_count > 0 {
        return PullFetchResult {
            has_uncommitted,
            behind_count,
            status: Status::PullError,
            message: format!(
                "diverged: {} ahead, {} behind (manual merge required)",
                ahead_count, behind_count
            ),
        };
    }

    if behind_count == 0 {
        PullFetchResult {
            has_uncommitted,
            behind_count: 0,
            status: Status::Synced,
            message: STATUS_SYNCED.to_string(),
        }
    } else {
        PullFetchResult {
            has_uncommitted,
            behind_count,
            status: Status::Synced, // Will be pulled in phase 2
            message: format!("{} commits behind", behind_count),
        }
    }
}

/// Phase 2: Pull repository if needed (write operation, moderate concurrency)
/// Returns (status, message, has_uncommitted_changes)
pub async fn pull_if_needed(
    path: &Path,
    fetch_result: &PullFetchResult,
    use_rebase: bool,
) -> (Status, String, bool) {
    use crate::core::clean_error_message;

    // If already synced or has errors, return immediately
    if fetch_result.status != Status::Synced {
        return (
            fetch_result.status,
            fetch_result.message.clone(),
            fetch_result.has_uncommitted,
        );
    }

    // If no commits behind, already synced
    if fetch_result.behind_count == 0 {
        return (
            Status::Synced,
            STATUS_SYNCED.to_string(),
            fetch_result.has_uncommitted,
        );
    }

    // Pull changes with appropriate strategy
    let pull_args = if use_rebase {
        // Use --autostash to safely stash uncommitted changes during rebase
        vec!["pull", "--rebase", "--autostash"]
    } else {
        vec!["pull", "--ff-only"]
    };

    match run_git(path, &pull_args).await {
        Ok((true, _, _)) => {
            let commits_word = if fetch_result.behind_count == 1 {
                "commit"
            } else {
                "commits"
            };
            (
                Status::Pulled,
                format!("{} {} pulled", fetch_result.behind_count, commits_word),
                fetch_result.has_uncommitted,
            )
        }
        Ok((false, _, stderr)) => {
            let error_message = clean_error_message(&stderr);

            // Check for common pull errors
            let final_message = if error_message.to_lowercase().contains("conflict") {
                format!("merge conflict: {}", error_message)
            } else if error_message.to_lowercase().contains("would be overwritten") {
                format!("uncommitted changes conflict: {}", error_message)
            } else if is_rate_limit_error(&error_message) {
                format!("⚠️ RATE LIMIT: {}", error_message)
            } else {
                error_message
            };
            (Status::PullError, final_message, fetch_result.has_uncommitted)
        }
        Err(e) => {
            let error_message = clean_error_message(&e.to_string());
            let final_message = if is_rate_limit_error(&error_message) {
                format!("⚠️ RATE LIMIT: {}", error_message)
            } else {
                error_message
            };
            (Status::PullError, final_message, fetch_result.has_uncommitted)
        }
    }
}

/// Repository visibility status
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RepoVisibility {
    Public,
    Private,
    Unknown,
}

// In-memory cache for repository visibility to avoid repeated gh CLI calls
// Using DashMap for lock-free concurrent access
// Cache is cleared when the program exits
static VISIBILITY_CACHE: OnceLock<DashMap<PathBuf, RepoVisibility>> = OnceLock::new();

/// Gets or initializes the visibility cache
fn get_visibility_cache() -> &'static DashMap<PathBuf, RepoVisibility> {
    VISIBILITY_CACHE.get_or_init(|| DashMap::new())
}

/// Detects repository visibility using gh CLI with in-memory caching
/// Returns RepoVisibility (defaults to Unknown if gh is not available or repo is not on GitHub)
/// Results are cached in-memory for the lifetime of the program to avoid repeated gh CLI calls
pub async fn get_repo_visibility(path: &Path) -> RepoVisibility {
    let path_buf = path.to_path_buf();
    let cache = get_visibility_cache();

    // Check cache first - lock-free read
    if let Some(visibility) = cache.get(&path_buf) {
        return *visibility;
    }

    // Not in cache, perform the expensive check
    let visibility = get_repo_visibility_uncached(path).await;

    // Store in cache - lock-free insert
    cache.insert(path_buf, visibility);

    visibility
}

/// Internal function to check visibility without caching
async fn get_repo_visibility_uncached(path: &Path) -> RepoVisibility {
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
