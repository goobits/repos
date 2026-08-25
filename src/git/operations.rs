//! Basic git operations and command execution

use anyhow::Result;
use dashmap::DashMap;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::process::Command;

use super::failure::{GitFailure, GitOperationPhase, GitOperationResult};
use super::remote::{inspect_remote, policy_violation, RemoteContext, RemoteDirection};
pub(crate) use super::runner::run_git;
use super::runner::run_git_raw;
use super::status::Status;
use super::worktree::{inspect_refreshed_repository_state, inspect_repository_state, HeadState};

// Git command arguments
const GIT_REMOTE_ARGS: &[&str] = &["remote"];
const GIT_REV_PARSE_HEAD_ARGS: &[&str] = &["rev-parse", "--abbrev-ref", "HEAD"];
const GIT_FETCH_ARGS: &[&str] = &["fetch", "--quiet"];
const GIT_REMOTE_REFS_ARGS: &[&str] = &[
    "for-each-ref",
    "--format=%(refname) %(objectname)",
    "refs/remotes",
    "refs/tags",
];
const GIT_CONFIG_GET_ARGS: &[&str] = &["config", "--get"];
const GIT_ADD_ARGS: &[&str] = &["add"];
const GIT_RESTORE_STAGED_ARGS: &[&str] = &["restore", "--staged"];
const GIT_STATUS_PORCELAIN_ARGS: &[&str] = &[
    "status",
    "--porcelain=v1",
    "--untracked-files=normal",
    "--ignore-submodules=dirty",
];
const GIT_COMMIT_ARGS: &[&str] = &["commit", "-m"];
const GIT_DIFF_CACHED_ARGS: &[&str] = &["diff", "--cached", "--quiet"];
const GIT_LFS_ENV_ARGS: &[&str] = &["lfs", "env"];

// Status messages
const DETACHED_HEAD_BRANCH: &str = "HEAD";
const STATUS_NO_REMOTE: &str = "no remote";
const STATUS_DETACHED_HEAD: &str = "detached HEAD";
const STATUS_NO_UPSTREAM: &str = "no tracking";
const STATUS_SYNCED: &str = "up to date";

/// Reads a git config value from the specified repository
/// Returns the config value if it exists, None if not found
pub(crate) async fn get_git_config(path: &Path, key: &str) -> Result<Option<String>> {
    let mut args = Vec::from(GIT_CONFIG_GET_ARGS);
    args.push(key);

    match run_git_raw(path, &args).await {
        Ok(output) if output.success() => {
            let value = output.stdout_text();
            if value.is_empty() {
                Ok(None)
            } else {
                Ok(Some(value))
            }
        }
        Ok(output) if output.exit_code == Some(1) && output.stderr.is_empty() => Ok(None),
        Ok(output) => {
            let stderr = output.stderr_text();
            anyhow::bail!(
                "{}",
                if stderr.is_empty() {
                    format!("git config failed with exit code {:?}", output.exit_code)
                } else {
                    stderr
                }
            )
        }
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
/// Returns (success, `error_message`)
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
        Err(e) => (false, format!("LFS error: {e}")),
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
    pub behind_count: u32,
    pub upstream_exists: bool,
    pub upstream_remote: Option<String>,
    pub upstream_branch: Option<String>,
    pub status: Status,
    pub message: String,
    pub(crate) failure: Option<GitFailure>,
}

impl FetchResult {
    fn error(message: String, has_uncommitted: bool, current_branch: String) -> Self {
        Self {
            has_uncommitted,
            current_branch,
            ahead_count: 0,
            behind_count: 0,
            upstream_exists: false,
            upstream_remote: None,
            upstream_branch: None,
            status: Status::Error,
            message,
            failure: None,
        }
    }

    fn failed(failure: GitFailure, has_uncommitted: bool, current_branch: String) -> Self {
        Self {
            has_uncommitted,
            current_branch,
            ahead_count: 0,
            behind_count: 0,
            upstream_exists: false,
            upstream_remote: None,
            upstream_branch: None,
            status: Status::Error,
            message: failure.message.clone(),
            failure: Some(failure),
        }
    }

    pub(crate) fn will_push(&self, auto_upstream: bool) -> bool {
        self.status == Status::Synced && self.ahead_count > 0
            || self.status == Status::NoUpstream && auto_upstream
    }
}

fn command_error(stderr: &str, fallback: &str) -> String {
    if stderr.trim().is_empty() {
        fallback.to_string()
    } else {
        crate::core::clean_error_message(stderr)
    }
}

fn result_from_fetch_state(
    status: Status,
    message: &str,
    has_uncommitted: bool,
    failure: Option<GitFailure>,
) -> GitOperationResult {
    let failure = failure.or_else(|| {
        matches!(
            status,
            Status::Error
                | Status::ConfigError
                | Status::StagingError
                | Status::CommitError
                | Status::PullError
        )
        .then(|| GitFailure::from_message(GitOperationPhase::Fetch, message.to_string(), None))
    });

    failure.map_or_else(
        || GitOperationResult::new(status, message.to_string(), has_uncommitted),
        |failure| GitOperationResult::failed(status, failure, has_uncommitted),
    )
}

/// Resolve the configured upstream remote + branch for the current branch.
async fn get_upstream_push_target(
    path: &Path,
    current_branch: &str,
) -> Result<Option<(String, String)>> {
    let remote_key = format!("branch.{current_branch}.remote");
    let merge_key = format!("branch.{current_branch}.merge");

    let remote_name = get_git_config(path, &remote_key).await?;
    let merge_ref = get_git_config(path, &merge_key).await?;

    if let (Some(remote_name), Some(merge_ref)) = (remote_name, merge_ref) {
        let upstream_branch = merge_ref
            .strip_prefix("refs/heads/")
            .unwrap_or(&merge_ref)
            .to_string();
        return Ok(Some((remote_name, upstream_branch)));
    }

    let upstream_ref = run_git(path, &["rev-parse", "--abbrev-ref", "@{upstream}"]).await?;
    if upstream_ref.0 {
        if let Some((remote_name, upstream_branch)) = upstream_ref.1.split_once('/') {
            return Ok(Some((remote_name.to_string(), upstream_branch.to_string())));
        }
    }

    Ok(None)
}

async fn get_branch_remote_name(
    path: &Path,
    current_branch: &str,
    remotes: &str,
) -> Result<String> {
    let remote_key = format!("branch.{current_branch}.remote");
    if let Some(remote) = get_git_config(path, &remote_key).await? {
        return Ok(remote);
    }

    Ok(remotes.lines().next().unwrap_or("origin").to_string())
}

async fn inspect_operation_remote(
    path: &Path,
    remote: &str,
    direction: RemoteDirection,
) -> Result<(Option<RemoteContext>, Option<GitFailure>)> {
    let contexts = inspect_remote(path, remote, direction).await?;
    let failure = policy_violation(&contexts)?.map(GitFailure::from_policy);
    Ok((contexts.into_iter().next(), failure))
}

async fn remote_ref_snapshot(path: &Path) -> Result<HashMap<String, String>> {
    match run_git(path, GIT_REMOTE_REFS_ARGS).await {
        Ok((true, output, _)) => Ok(output
            .lines()
            .filter_map(|line| {
                line.split_once(' ')
                    .map(|(name, object)| (name.to_string(), object.to_string()))
            })
            .collect()),
        Ok((false, _, stderr)) => Err(anyhow::anyhow!(command_error(
            &stderr,
            "remote reference inspection failed"
        ))),
        Err(error) => Err(error),
    }
}

fn count_ref_updates(before: &HashMap<String, String>, after: &HashMap<String, String>) -> u64 {
    after
        .iter()
        .filter(|(name, object)| before.get(*name) != Some(*object))
        .count() as u64
}

/// Fetches every configured remote without changing the branch or worktree.
pub(crate) async fn fetch_remote_updates(path: &Path) -> GitOperationResult {
    use crate::core::clean_error_message;

    let remotes = match run_git(path, GIT_REMOTE_ARGS).await {
        Ok((true, output, _)) => output,
        Ok((false, _, stderr)) => {
            return GitOperationResult::new(
                Status::Error,
                command_error(&stderr, "remote inspection failed"),
                false,
            )
        }
        Err(error) => {
            return GitOperationResult::new(
                Status::Error,
                clean_error_message(&error.to_string()),
                false,
            )
        }
    };
    if remotes.trim().is_empty() {
        return GitOperationResult::new(Status::NoRemote, STATUS_NO_REMOTE.to_string(), false);
    }

    let mut fetch_remotes = Vec::new();
    for remote in remotes.lines().filter(|remote| !remote.trim().is_empty()) {
        match inspect_operation_remote(path, remote, RemoteDirection::Fetch).await {
            Ok((_, Some(failure))) => {
                return GitOperationResult::failed(Status::Error, failure, false);
            }
            Ok((context, None)) => {
                fetch_remotes.push((remote, context));
            }
            Err(error) => {
                let failure = GitFailure::from_message(
                    GitOperationPhase::RemoteInspection,
                    format!("remote inspection failed: {error}"),
                    None,
                );
                return GitOperationResult::failed(Status::Error, failure, false);
            }
        }
    }

    let before = match remote_ref_snapshot(path).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let failure = GitFailure::from_message(
                GitOperationPhase::Fetch,
                clean_error_message(&error.to_string()),
                None,
            );
            return GitOperationResult::failed(Status::Error, failure, false);
        }
    };

    for (remote, context) in fetch_remotes {
        let fetch_error = match run_git(path, &["fetch", "--quiet", remote]).await {
            Ok((true, _, _)) => None,
            Ok((false, _, stderr)) => Some(command_error(&stderr, "fetch failed")),
            Err(error) => Some(clean_error_message(&error.to_string())),
        };
        if let Some(message) = fetch_error {
            let message = if is_rate_limit_error(&message) {
                format!("⚠️ RATE LIMIT: {message}")
            } else {
                message
            };
            let failure = GitFailure::from_message(GitOperationPhase::Fetch, message, context);
            return GitOperationResult::failed(Status::Error, failure, false);
        }
    }

    let after = match remote_ref_snapshot(path).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let failure = GitFailure::from_message(
                GitOperationPhase::Fetch,
                clean_error_message(&error.to_string()),
                None,
            );
            return GitOperationResult::failed(Status::Error, failure, false);
        }
    };
    let updated_refs = count_ref_updates(&before, &after);
    if updated_refs == 0 {
        GitOperationResult::new(Status::Synced, STATUS_SYNCED.to_string(), false)
    } else {
        let label = if updated_refs == 1 { "ref" } else { "refs" };
        GitOperationResult::new(
            Status::Fetched,
            format!("{updated_refs} remote {label} updated"),
            false,
        )
    }
}

/// Fetches configured remotes and verifies that a commit is reachable from a
/// remote-tracking branch. Local tags are intentionally excluded because Git
/// does not record whether a tag has actually been pushed.
pub(crate) async fn remote_refs_contain_commit(path: &Path, commit: &str) -> Result<bool> {
    let remotes = match run_git(path, GIT_REMOTE_ARGS).await {
        Ok((true, remotes, _)) => remotes,
        Ok((false, _, stderr)) => {
            anyhow::bail!(command_error(&stderr, "remote inspection failed"))
        }
        Err(error) => return Err(error),
    };
    if remotes.trim().is_empty() {
        anyhow::bail!(STATUS_NO_REMOTE);
    }

    let mut fetched_any = false;
    let mut failures = Vec::new();
    let contains = format!("--contains={commit}");
    for remote in remotes.lines().filter(|remote| !remote.trim().is_empty()) {
        match inspect_operation_remote(path, remote, RemoteDirection::Fetch).await {
            Ok((_, Some(failure))) => {
                failures.push(failure.message);
                continue;
            }
            Ok((_, None)) => {}
            Err(error) => {
                failures.push(format!("{remote}: {error}"));
                continue;
            }
        }

        match run_git(path, &["fetch", "--quiet", remote]).await {
            Ok((true, _, _)) => fetched_any = true,
            Ok((false, _, stderr)) => {
                failures.push(format!(
                    "{remote}: {}",
                    command_error(&stderr, "fetch failed")
                ));
                continue;
            }
            Err(error) => {
                failures.push(format!("{remote}: {error}"));
                continue;
            }
        }

        let namespace = format!("refs/remotes/{remote}");
        match run_git(
            path,
            &["for-each-ref", &contains, "--format=%(refname)", &namespace],
        )
        .await
        {
            Ok((true, refs, _)) if !refs.trim().is_empty() => return Ok(true),
            Ok((true, _, _)) => {}
            Ok((false, _, stderr)) => failures.push(format!(
                "{remote}: {}",
                command_error(&stderr, "remote reachability check failed")
            )),
            Err(error) => failures.push(format!("{remote}: {error}")),
        }
    }

    if fetched_any {
        Ok(false)
    } else {
        anyhow::bail!(failures.join("; "))
    }
}

pub(crate) async fn unpublished_gitlinks(
    prerequisites: &[crate::core::GitlinkPrerequisite],
) -> Vec<String> {
    let mut unpublished = Vec::new();
    for prerequisite in prerequisites {
        if !matches!(
            remote_refs_contain_commit(&prerequisite.path, &prerequisite.target).await,
            Ok(true)
        ) {
            unpublished.push(prerequisite.name.clone());
        }
    }
    unpublished
}

/// Phase 1: Fetch and analyze repository state (read-only, can be highly concurrent)
/// Returns `FetchResult` with repository state after fetching
pub async fn fetch_and_analyze(path: &Path, _auto_upstream: bool) -> FetchResult {
    use crate::core::clean_error_message;

    let initial_state = match inspect_refreshed_repository_state(path).await {
        Ok(state) => state,
        Err(e) => {
            return FetchResult::error(clean_error_message(&e.to_string()), false, String::new())
        }
    };
    let has_uncommitted = initial_state.is_dirty();
    let current_branch = match initial_state.head() {
        HeadState::Branch(branch) => branch.clone(),
        HeadState::Unborn => "HEAD".to_string(),
        HeadState::Detached => {
            return FetchResult {
                has_uncommitted,
                current_branch: String::new(),
                ahead_count: 0,
                behind_count: 0,
                upstream_exists: false,
                upstream_remote: None,
                upstream_branch: None,
                status: Status::Skip,
                message: STATUS_DETACHED_HEAD.to_string(),
                failure: None,
            }
        }
        HeadState::Unknown => {
            return FetchResult::error(
                "branch inspection returned no HEAD state".to_string(),
                has_uncommitted,
                String::new(),
            )
        }
    };

    // Get list of remotes
    let remotes = match run_git(path, GIT_REMOTE_ARGS).await {
        Ok((true, output, _)) => output,
        Ok((false, _, stderr)) => {
            return FetchResult::error(
                command_error(&stderr, "remote inspection failed"),
                has_uncommitted,
                String::new(),
            )
        }
        Err(e) => {
            return FetchResult::error(
                clean_error_message(&e.to_string()),
                has_uncommitted,
                String::new(),
            )
        }
    };

    if remotes.trim().is_empty() {
        return FetchResult {
            has_uncommitted,
            current_branch: String::new(),
            ahead_count: 0,
            behind_count: 0,
            upstream_exists: false,
            upstream_remote: None,
            upstream_branch: None,
            status: Status::NoRemote,
            message: STATUS_NO_REMOTE.to_string(),
            failure: None,
        };
    }

    let fetch_remote = match get_branch_remote_name(path, &current_branch, &remotes).await {
        Ok(remote) => remote,
        Err(error) => {
            return FetchResult::error(
                clean_error_message(&error.to_string()),
                has_uncommitted,
                current_branch,
            )
        }
    };
    let (fetch_context, policy_failure) =
        match inspect_operation_remote(path, &fetch_remote, RemoteDirection::Fetch).await {
            Ok(result) => result,
            Err(error) => {
                let failure = GitFailure::from_message(
                    GitOperationPhase::RemoteInspection,
                    format!("remote inspection failed: {error}"),
                    None,
                );
                return FetchResult::failed(failure, has_uncommitted, current_branch);
            }
        };
    if let Some(failure) = policy_failure {
        return FetchResult::failed(failure, has_uncommitted, current_branch);
    }

    // Fetch latest changes to ensure we have up-to-date refs
    let fetch_error = match run_git(path, GIT_FETCH_ARGS).await {
        Ok((true, _, _)) => None,
        Ok((false, _, stderr)) => Some(command_error(&stderr, "fetch failed")),
        Err(e) => Some(clean_error_message(&e.to_string())),
    };
    if let Some(error_message) = fetch_error {
        let final_message = if is_rate_limit_error(&error_message) {
            format!("⚠️ RATE LIMIT: {error_message}")
        } else {
            error_message
        };
        let failure =
            GitFailure::from_message(GitOperationPhase::Fetch, final_message, fetch_context);
        return FetchResult::failed(failure, has_uncommitted, current_branch);
    }

    let final_state = match inspect_repository_state(path).await {
        Ok(state) => state,
        Err(error) => {
            return FetchResult::error(
                clean_error_message(&error.to_string()),
                has_uncommitted,
                current_branch,
            )
        }
    };
    if final_state.head() != &HeadState::Branch(current_branch.clone()) {
        return FetchResult::error(
            "branch changed while repository state was being inspected".to_string(),
            has_uncommitted,
            current_branch,
        );
    }
    let Some(upstream_name) = final_state.upstream() else {
        // May be pushed in phase 2 if the caller opted into setting upstreams.
        let status = Status::NoUpstream;
        return FetchResult {
            has_uncommitted,
            current_branch,
            ahead_count: 0,
            behind_count: 0,
            upstream_exists: false,
            upstream_remote: Some(fetch_remote),
            upstream_branch: None,
            status,
            message: STATUS_NO_UPSTREAM.to_string(),
            failure: None,
        };
    };
    let Some(counts) = final_state.ahead_behind() else {
        return FetchResult::error(
            "upstream ancestry counts are missing".to_string(),
            has_uncommitted,
            current_branch,
        );
    };
    let upstream_remote = Some(fetch_remote.clone());
    let upstream_branch = upstream_name
        .strip_prefix(&format!("{fetch_remote}/"))
        .map(str::to_string);
    let upstream_branch = match upstream_branch {
        Some(branch) => Some(branch),
        None => match get_upstream_push_target(path, &current_branch).await {
            Ok(Some((_, branch))) => Some(branch),
            Ok(None) => {
                return FetchResult::error(
                    "configured upstream branch could not be resolved".to_string(),
                    has_uncommitted,
                    current_branch,
                )
            }
            Err(e) => {
                return FetchResult::error(
                    clean_error_message(&e.to_string()),
                    has_uncommitted,
                    current_branch,
                )
            }
        },
    };

    let ahead_count = counts.ahead;
    let behind_count = counts.behind;

    // Branches have diverged - both ahead and behind
    if ahead_count > 0 && behind_count > 0 {
        return FetchResult {
            has_uncommitted,
            current_branch,
            ahead_count,
            behind_count,
            upstream_exists: true,
            upstream_remote,
            upstream_branch,
            status: Status::Error,
            message: format!(
                "diverged: {ahead_count} ahead, {behind_count} behind (run repos sync or resolve manually)"
            ),
            failure: None,
        };
    }

    if ahead_count == 0 {
        FetchResult {
            has_uncommitted,
            current_branch,
            ahead_count: 0,
            behind_count,
            upstream_exists: true,
            upstream_remote,
            upstream_branch,
            status: Status::Synced,
            message: STATUS_SYNCED.to_string(),
            failure: None,
        }
    } else {
        FetchResult {
            has_uncommitted,
            current_branch,
            ahead_count,
            behind_count,
            upstream_exists: true,
            upstream_remote,
            upstream_branch,
            status: Status::Synced, // Will be pushed in phase 2
            message: format!("{ahead_count} commits ahead"),
            failure: None,
        }
    }
}

/// Phase 2: Push repository if needed (write operation, moderate concurrency)
/// Returns (status, message, `has_uncommitted_changes`)
pub async fn push_if_needed(
    path: &Path,
    fetch_result: &FetchResult,
    auto_upstream: bool,
) -> (Status, String, bool) {
    push_if_needed_with_context(path, fetch_result, auto_upstream)
        .await
        .into_tuple()
}

/// Internal push entry point that retains safe remote context for reporting.
pub(crate) async fn push_if_needed_with_context(
    path: &Path,
    fetch_result: &FetchResult,
    auto_upstream: bool,
) -> GitOperationResult {
    use crate::core::clean_error_message;

    // If already synced or has errors, return immediately
    if fetch_result.status != Status::Synced && fetch_result.status != Status::NoUpstream {
        return result_from_fetch_state(
            fetch_result.status,
            &fetch_result.message,
            fetch_result.has_uncommitted,
            fetch_result.failure.clone(),
        );
    }

    // Fetch already validated the transport and captured an exact ancestry
    // snapshot. A tracked branch with nothing ahead cannot perform a push, so
    // avoid repeating remote, push-URL, and LFS inspection for this no-op case.
    if fetch_result.upstream_exists && fetch_result.ahead_count == 0 {
        return GitOperationResult::new(
            Status::Synced,
            STATUS_SYNCED.to_string(),
            fetch_result.has_uncommitted,
        );
    }

    // Fetch captured the exact branch/default remote used for this operation.
    let remote_name = fetch_result
        .upstream_remote
        .clone()
        .unwrap_or_else(|| "origin".to_string());
    let target_branch = fetch_result
        .upstream_branch
        .clone()
        .unwrap_or_else(|| fetch_result.current_branch.clone());

    let (push_context, policy_failure) =
        match inspect_operation_remote(path, &remote_name, RemoteDirection::Push).await {
            Ok(result) => result,
            Err(error) => {
                let failure = GitFailure::from_message(
                    GitOperationPhase::RemoteInspection,
                    format!("remote inspection failed: {error}"),
                    None,
                );
                return GitOperationResult::failed(
                    Status::Error,
                    failure,
                    fetch_result.has_uncommitted,
                );
            }
        };
    if let Some(failure) = policy_failure {
        return GitOperationResult::failed(Status::Error, failure, fetch_result.has_uncommitted);
    }

    // Check if repo uses Git LFS and push LFS objects FIRST
    let uses_lfs = check_uses_git_lfs(path).await;
    if uses_lfs && has_pending_lfs_objects(path).await {
        let branch = if target_branch.is_empty() {
            "HEAD".to_string()
        } else {
            target_branch.clone()
        };

        let (lfs_success, lfs_error) = push_lfs_objects(path, &remote_name, &branch).await;
        if !lfs_success {
            // LFS push failed - return error (use default message if error is empty)
            let error_msg = if lfs_error.is_empty() {
                "LFS push failed".to_string()
            } else {
                lfs_error
            };
            let failure = GitFailure::from_message(
                GitOperationPhase::LfsPush,
                error_msg,
                push_context.clone(),
            );
            return GitOperationResult::failed(
                Status::Error,
                failure,
                fetch_result.has_uncommitted,
            );
        }
    }

    // Handle no upstream case
    if !fetch_result.upstream_exists {
        if auto_upstream {
            let push_args = vec!["push", "-u", &remote_name, &fetch_result.current_branch];
            match run_git(path, &push_args).await {
                Ok((true, _, _)) => {
                    let msg = if uses_lfs {
                        format!("set upstream ({remote_name}) & pushed (with LFS)")
                    } else {
                        format!("set upstream ({remote_name}) & pushed")
                    };
                    return GitOperationResult::new(
                        Status::Pushed,
                        msg,
                        fetch_result.has_uncommitted,
                    );
                }
                Ok((false, _, stderr)) => {
                    let error_message = clean_error_message(&stderr);
                    let failure = GitFailure::from_message(
                        GitOperationPhase::Push,
                        error_message,
                        push_context,
                    );
                    return GitOperationResult::failed(
                        Status::Error,
                        failure,
                        fetch_result.has_uncommitted,
                    );
                }
                Err(e) => {
                    let error_message = clean_error_message(&e.to_string());
                    let failure = GitFailure::from_message(
                        GitOperationPhase::Push,
                        error_message,
                        push_context,
                    );
                    return GitOperationResult::failed(
                        Status::Error,
                        failure,
                        fetch_result.has_uncommitted,
                    );
                }
            }
        }
        return GitOperationResult::new(
            Status::NoUpstream,
            STATUS_NO_UPSTREAM.to_string(),
            fetch_result.has_uncommitted,
        );
    }

    // If no commits ahead, already synced
    if fetch_result.ahead_count == 0 {
        return GitOperationResult::new(
            Status::Synced,
            STATUS_SYNCED.to_string(),
            fetch_result.has_uncommitted,
        );
    }

    // Push changes - use explicit remote and branch to match LFS push
    let push_refspec = if target_branch == fetch_result.current_branch {
        fetch_result.current_branch.clone()
    } else {
        format!("{}:{}", fetch_result.current_branch, target_branch)
    };
    let push_args = vec!["push", &remote_name, &push_refspec];
    match run_git(path, &push_args).await {
        Ok((true, _, _)) => {
            let commits_word = if fetch_result.ahead_count == 1 {
                "commit"
            } else {
                "commits"
            };
            let msg = if uses_lfs {
                format!(
                    "{} {} pushed (with LFS)",
                    fetch_result.ahead_count, commits_word
                )
            } else {
                format!("{} {} pushed", fetch_result.ahead_count, commits_word)
            };
            GitOperationResult::new(Status::Pushed, msg, fetch_result.has_uncommitted)
        }
        Ok((false, _, stderr)) => {
            let error_message = clean_error_message(&stderr);
            let final_message = if is_rate_limit_error(&error_message) {
                format!("⚠️ RATE LIMIT: {error_message}")
            } else {
                error_message
            };
            let failure =
                GitFailure::from_message(GitOperationPhase::Push, final_message, push_context);
            GitOperationResult::failed(Status::Error, failure, fetch_result.has_uncommitted)
        }
        Err(e) => {
            let error_message = clean_error_message(&e.to_string());
            let final_message = if is_rate_limit_error(&error_message) {
                format!("⚠️ RATE LIMIT: {error_message}")
            } else {
                error_message
            };
            let failure =
                GitFailure::from_message(GitOperationPhase::Push, final_message, push_context);
            GitOperationResult::failed(Status::Error, failure, fetch_result.has_uncommitted)
        }
    }
}

/// Stages tracked modifications and deletions only.
///
/// This intentionally does not stage untracked files. It is the safe default for
/// fleet-wide workflows such as `repos save`.
pub async fn stage_tracked_changes(path: &Path) -> Result<(bool, String, String)> {
    run_git(path, &["add", "-u"]).await
}

/// Stages all non-ignored changes, including untracked files.
pub async fn stage_all_changes(path: &Path) -> Result<(bool, String, String)> {
    run_git(path, &["add", "-A"]).await
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
        Ok((true, stdout, stderr)) => Ok((stdout, stderr)),
        Ok((false, _, stderr)) => Err(anyhow::anyhow!(command_error(
            &stderr,
            "status inspection failed"
        ))),
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

/// Checks if a repository has uncommitted changes.
///
/// Uses NUL-delimited porcelain-v2 output so unusual and non-UTF-8 paths cannot
/// corrupt record boundaries.
///
/// Note: There are synchronous versions in subrepo/{mod.rs, sync.rs} for use
/// in non-async contexts.
pub async fn has_uncommitted_changes(path: &Path) -> Result<bool> {
    // Refresh tracked file stat info first. Ignore failures because status below
    // is authoritative and works for unborn repositories.
    let _ = run_git(path, &["update-index", "--refresh"]).await;

    Ok(super::worktree::inspect_worktree(path).await?.is_dirty())
}

/// Returns true when the repository is on a detached HEAD.
pub async fn is_detached_head(path: &Path) -> Result<bool> {
    match run_git(path, GIT_REV_PARSE_HEAD_ARGS).await {
        Ok((true, branch, _)) => Ok(branch == DETACHED_HEAD_BRANCH),
        Ok((false, _, stderr)) => Err(anyhow::anyhow!(stderr)),
        Err(e) => Err(e),
    }
}

/// Creates a git tag and pushes it to the remote
/// Returns (success, message)
pub async fn create_and_push_tag(path: &Path, tag_name: &str) -> (bool, String) {
    let tag_ref = format!("refs/tags/{tag_name}^{{commit}}");
    let existing_target = run_git(path, &["rev-parse", "--verify", &tag_ref]).await;
    if let Ok((true, existing_target, _)) = &existing_target {
        let head = match run_git(path, &["rev-parse", "HEAD"]).await {
            Ok((true, head, _)) => head,
            Ok((false, _, stderr)) => {
                return (false, format!("failed to resolve release commit: {stderr}"))
            }
            Err(error) => return (false, format!("failed to resolve release commit: {error}")),
        };
        if existing_target != &head {
            return (
                false,
                format!(
                    "existing tag {tag_name} points to {}, not release commit {}",
                    existing_target.chars().take(7).collect::<String>(),
                    head.chars().take(7).collect::<String>()
                ),
            );
        }
    }

    let tag_result = run_git(path, &["tag", "--", tag_name]).await;

    let (success, _, stderr) = match tag_result {
        Ok(result) => result,
        Err(e) => return (false, format!("failed to create tag: {e}")),
    };

    let existed = !success && stderr.contains("already exists");
    if !success && !existed {
        return (false, format!("failed to create tag: {stderr}"));
    }

    let current_branch = match run_git(path, GIT_REV_PARSE_HEAD_ARGS).await {
        Ok((true, branch, _)) if branch != DETACHED_HEAD_BRANCH => Some(branch),
        _ => None,
    };
    let upstream_remote = if let Some(branch) = current_branch {
        get_upstream_push_target(path, &branch)
            .await
            .ok()
            .flatten()
            .map(|(remote, _)| remote)
    } else {
        None
    };
    let remote_name = match upstream_remote {
        Some(remote) => remote,
        None => match run_git(path, GIT_REMOTE_ARGS).await {
            Ok((true, remotes, _)) => {
                let names = remotes.lines().collect::<Vec<_>>();
                if names.contains(&"origin") {
                    "origin".to_string()
                } else if let Some(remote) = names.first() {
                    (*remote).to_string()
                } else {
                    return (
                        false,
                        "tag created locally; no remote configured".to_string(),
                    );
                }
            }
            Ok((false, _, stderr)) => {
                return (
                    false,
                    format!("tag created locally; remote inspection failed: {stderr}"),
                )
            }
            Err(e) => {
                return (
                    false,
                    format!("tag created locally; remote inspection failed: {e}"),
                )
            }
        },
    };

    match inspect_operation_remote(path, &remote_name, RemoteDirection::Push).await {
        Ok((_, Some(failure))) => {
            return (false, format!("tag created locally; {}", failure.message));
        }
        Ok((_, None)) => {}
        Err(error) => {
            return (
                false,
                format!("tag created locally; remote inspection failed: {error}"),
            );
        }
    }

    let tag_refspec = format!("refs/tags/{tag_name}:refs/tags/{tag_name}");
    let push_result = run_git(path, &["push", &remote_name, &tag_refspec]).await;

    match push_result {
        Ok((true, _, _)) if existed => (true, format!("existing tag pushed {tag_name}")),
        Ok((true, _, _)) => (true, format!("tagged & pushed {tag_name}")),
        Ok((false, _, stderr)) => (
            false,
            format!(
                "tag exists locally; push failed: {}",
                stderr.lines().next().unwrap_or("unknown error")
            ),
        ),
        Err(e) => (false, format!("tag exists locally; push failed: {e}")),
    }
}

/// Result of the fetch phase for pull operation
#[derive(Clone)]
pub struct PullFetchResult {
    pub has_uncommitted: bool,
    pub ahead_count: u32,
    pub behind_count: u32,
    pub upstream_name: Option<String>,
    pub status: Status,
    pub message: String,
    pub(crate) failure: Option<GitFailure>,
    pub(crate) remote: Option<RemoteContext>,
}

impl PullFetchResult {
    fn error(message: String, has_uncommitted: bool) -> Self {
        Self {
            has_uncommitted,
            ahead_count: 0,
            behind_count: 0,
            upstream_name: None,
            status: Status::Error,
            message,
            failure: None,
            remote: None,
        }
    }

    fn failed(failure: GitFailure, has_uncommitted: bool) -> Self {
        Self {
            has_uncommitted,
            ahead_count: 0,
            behind_count: 0,
            upstream_name: None,
            status: Status::Error,
            message: failure.message.clone(),
            failure: Some(failure),
            remote: None,
        }
    }
}

/// Phase 1: Fetch and analyze repository state for pull (read-only, can be highly concurrent)
/// Returns `PullFetchResult` with repository state after fetching
pub async fn fetch_and_analyze_for_pull(path: &Path) -> PullFetchResult {
    use crate::core::clean_error_message;

    let initial_state = match inspect_refreshed_repository_state(path).await {
        Ok(state) => state,
        Err(e) => {
            return PullFetchResult::error(clean_error_message(&e.to_string()), false);
        }
    };
    fetch_and_analyze_for_pull_with_state(path, initial_state).await
}

pub(crate) async fn fetch_and_analyze_for_pull_with_state(
    path: &Path,
    initial_state: super::worktree::WorktreeState,
) -> PullFetchResult {
    use crate::core::clean_error_message;

    let has_uncommitted = initial_state.is_dirty();
    let current_branch = match initial_state.head() {
        HeadState::Branch(branch) => branch.clone(),
        HeadState::Unborn => "HEAD".to_string(),
        HeadState::Detached => {
            return PullFetchResult {
                has_uncommitted,
                ahead_count: 0,
                behind_count: 0,
                upstream_name: None,
                status: Status::Skip,
                message: STATUS_DETACHED_HEAD.to_string(),
                failure: None,
                remote: None,
            }
        }
        HeadState::Unknown => {
            return PullFetchResult::error(
                "branch inspection returned no HEAD state".to_string(),
                has_uncommitted,
            )
        }
    };

    // Get list of remotes
    let remotes = match run_git(path, GIT_REMOTE_ARGS).await {
        Ok((true, output, _)) => output,
        Ok((false, _, stderr)) => {
            return PullFetchResult::error(
                command_error(&stderr, "remote inspection failed"),
                has_uncommitted,
            )
        }
        Err(e) => {
            return PullFetchResult::error(clean_error_message(&e.to_string()), has_uncommitted)
        }
    };

    if remotes.trim().is_empty() {
        return PullFetchResult {
            has_uncommitted,
            ahead_count: 0,
            behind_count: 0,
            upstream_name: None,
            status: Status::NoRemote,
            message: STATUS_NO_REMOTE.to_string(),
            failure: None,
            remote: None,
        };
    }

    let fetch_remote = match get_branch_remote_name(path, &current_branch, &remotes).await {
        Ok(remote) => remote,
        Err(error) => {
            return PullFetchResult::error(clean_error_message(&error.to_string()), has_uncommitted)
        }
    };
    let (fetch_context, policy_failure) =
        match inspect_operation_remote(path, &fetch_remote, RemoteDirection::Fetch).await {
            Ok(result) => result,
            Err(error) => {
                let failure = GitFailure::from_message(
                    GitOperationPhase::RemoteInspection,
                    format!("remote inspection failed: {error}"),
                    None,
                );
                return PullFetchResult::failed(failure, has_uncommitted);
            }
        };
    if let Some(failure) = policy_failure {
        return PullFetchResult::failed(failure, has_uncommitted);
    }

    // Fetch latest changes to ensure we have up-to-date refs
    let fetch_error = match run_git(path, GIT_FETCH_ARGS).await {
        Ok((true, _, _)) => None,
        Ok((false, _, stderr)) => Some(command_error(&stderr, "fetch failed")),
        Err(e) => Some(clean_error_message(&e.to_string())),
    };
    if let Some(error_message) = fetch_error {
        let final_message = if is_rate_limit_error(&error_message) {
            format!("⚠️ RATE LIMIT: {error_message}")
        } else {
            error_message
        };
        let failure =
            GitFailure::from_message(GitOperationPhase::Fetch, final_message, fetch_context);
        return PullFetchResult::failed(failure, has_uncommitted);
    }

    let final_state = match inspect_repository_state(path).await {
        Ok(state) => state,
        Err(error) => {
            return PullFetchResult::error(clean_error_message(&error.to_string()), has_uncommitted)
        }
    };
    if final_state.head() != &HeadState::Branch(current_branch) {
        return PullFetchResult::error(
            "branch changed while repository state was being inspected".to_string(),
            has_uncommitted,
        );
    }
    let upstream_name = final_state.upstream().map(str::to_string);
    if upstream_name.is_none() {
        return PullFetchResult {
            has_uncommitted,
            ahead_count: 0,
            behind_count: 0,
            upstream_name: None,
            status: Status::NoUpstream,
            message: STATUS_NO_UPSTREAM.to_string(),
            failure: None,
            remote: fetch_context,
        };
    }

    let Some(counts) = final_state.ahead_behind() else {
        return PullFetchResult::error(
            "upstream ancestry counts are missing".to_string(),
            has_uncommitted,
        );
    };
    let ahead_count = counts.ahead;
    let behind_count = counts.behind;

    // Branches have diverged - both ahead and behind
    if ahead_count > 0 && behind_count > 0 {
        return PullFetchResult {
            has_uncommitted,
            ahead_count,
            behind_count,
            upstream_name,
            status: Status::PullError,
            message: format!(
                "diverged: {ahead_count} ahead, {behind_count} behind (run repos sync or resolve manually)"
            ),
            failure: None,
            remote: fetch_context,
        };
    }

    if behind_count == 0 {
        PullFetchResult {
            has_uncommitted,
            ahead_count,
            behind_count: 0,
            upstream_name,
            status: Status::Synced,
            message: STATUS_SYNCED.to_string(),
            failure: None,
            remote: fetch_context,
        }
    } else {
        PullFetchResult {
            has_uncommitted,
            ahead_count,
            behind_count,
            upstream_name,
            status: Status::Synced, // Will be pulled in phase 2
            message: format!("{behind_count} commits behind"),
            failure: None,
            remote: fetch_context,
        }
    }
}

/// Phase 2: Pull repository if needed (write operation, moderate concurrency)
/// Returns (status, message, `has_uncommitted_changes`)
pub async fn pull_if_needed(
    path: &Path,
    fetch_result: &PullFetchResult,
    use_rebase: bool,
) -> (Status, String, bool) {
    pull_if_needed_with_context(path, fetch_result, use_rebase)
        .await
        .into_tuple()
}

/// Internal pull entry point that retains safe remote context for reporting.
pub(crate) async fn pull_if_needed_with_context(
    path: &Path,
    fetch_result: &PullFetchResult,
    use_rebase: bool,
) -> GitOperationResult {
    use crate::core::clean_error_message;

    // If already synced or has errors, return immediately
    if fetch_result.status != Status::Synced {
        return result_from_fetch_state(
            fetch_result.status,
            &fetch_result.message,
            fetch_result.has_uncommitted,
            fetch_result.failure.clone(),
        );
    }

    // If no commits behind, already synced
    if fetch_result.behind_count == 0 {
        return GitOperationResult::new(
            Status::Synced,
            STATUS_SYNCED.to_string(),
            fetch_result.has_uncommitted,
        );
    }

    if fetch_result.has_uncommitted {
        return GitOperationResult::new(
            Status::Skip,
            "dirty worktree; commit or stash before sync".to_string(),
            true,
        );
    }

    // Pre-fetch LFS objects if repo uses LFS (avoids delays during checkout)
    let uses_lfs = check_uses_git_lfs(path).await;
    if uses_lfs {
        // Fetch LFS objects for incoming commits - errors are non-fatal
        let _ = run_git(path, &["lfs", "fetch"]).await;
    }

    // Pull changes with appropriate strategy
    let pull_args = if use_rebase {
        vec!["pull", "--rebase"]
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
            GitOperationResult::new(
                Status::Pulled,
                format!(
                    "{} {} pulled{}",
                    fetch_result.behind_count,
                    commits_word,
                    if uses_lfs { " (with LFS)" } else { "" }
                ),
                fetch_result.has_uncommitted,
            )
        }
        Ok((false, _, stderr)) => {
            let error_message = clean_error_message(&stderr);

            // Check for common pull errors
            let final_message = if error_message.to_lowercase().contains("conflict") {
                format!("merge conflict: {error_message}")
            } else if error_message
                .to_lowercase()
                .contains("would be overwritten")
            {
                format!("uncommitted changes conflict: {error_message}")
            } else if is_rate_limit_error(&error_message) {
                format!("⚠️ RATE LIMIT: {error_message}")
            } else {
                error_message
            };
            let failure = GitFailure::from_message(
                GitOperationPhase::Pull,
                final_message,
                fetch_result.remote.clone(),
            );
            GitOperationResult::failed(Status::PullError, failure, fetch_result.has_uncommitted)
        }
        Err(e) => {
            let error_message = clean_error_message(&e.to_string());
            let final_message = if is_rate_limit_error(&error_message) {
                format!("⚠️ RATE LIMIT: {error_message}")
            } else {
                error_message
            };
            let failure = GitFailure::from_message(
                GitOperationPhase::Pull,
                final_message,
                fetch_result.remote.clone(),
            );
            GitOperationResult::failed(Status::PullError, failure, fetch_result.has_uncommitted)
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
    VISIBILITY_CACHE.get_or_init(DashMap::new)
}

/// Detects repository visibility using gh CLI with in-memory caching
/// Returns `RepoVisibility` (defaults to Unknown if gh is not available or repo is not on GitHub)
/// Results are cached in-memory for the lifetime of the program to avoid repeated gh CLI calls
pub async fn get_repo_visibility(path: &Path) -> RepoVisibility {
    let cache = get_visibility_cache();

    // Check cache first - lock-free read
    // Note: Use &Path lookup to avoid PathBuf allocation on cache hits
    if let Some(visibility) = cache.get(path) {
        return *visibility;
    }

    // Not in cache, perform the expensive check
    let visibility = get_repo_visibility_uncached(path).await;

    // Store in cache - lock-free insert
    cache.insert(path.to_path_buf(), visibility);

    visibility
}

/// Internal function to check visibility without caching
async fn get_repo_visibility_uncached(path: &Path) -> RepoVisibility {
    // First check if this is a GitHub repository by looking at the remote URL
    let remote_url = match run_git(path, &["remote", "get-url", "origin"]).await {
        Ok((true, url, _)) => url,
        _ => return RepoVisibility::Unknown,
    };

    if !super::remote::is_github_remote_url(&remote_url) {
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
