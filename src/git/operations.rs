//! Basic git operations and command execution

mod lfs;
mod pull;
mod remotes;
mod visibility;
mod worktree;

pub use lfs::{check_uses_git_lfs, has_pending_lfs_objects, push_lfs_objects};
pub use pull::{fetch_and_analyze_for_pull, pull_if_needed, PullFetchResult};
pub(crate) use pull::{fetch_and_analyze_for_pull_with_state, pull_if_needed_with_context};
pub(crate) use remotes::{fetch_remote_updates, unpublished_gitlinks};
pub use visibility::{get_repo_visibility, RepoVisibility};
pub use worktree::{
    commit_changes, create_and_push_tag, get_staging_status, has_staged_changes,
    has_uncommitted_changes, is_detached_head, stage_all_changes, stage_files,
    stage_tracked_changes, unstage_files,
};

use anyhow::Result;
use std::path::Path;

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
            let transferred =
                match count_new_upstream_commits(path, &remote_name, &target_branch).await {
                    Ok(count) => count,
                    Err(error) => {
                        let failure = GitFailure::from_message(
                            GitOperationPhase::Push,
                            format!("new-upstream commit inspection failed: {error}"),
                            push_context,
                        );
                        return GitOperationResult::failed(
                            Status::Error,
                            failure,
                            fetch_result.has_uncommitted,
                        );
                    }
                };
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
                    )
                    .with_transferred(transferred);
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
                .with_transferred(fetch_result.ahead_count.into())
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

async fn count_new_upstream_commits(path: &Path, remote: &str, branch: &str) -> Result<u64> {
    let remote_ref = format!("refs/remotes/{remote}/{branch}");
    let reference_exists =
        match run_git(path, &["show-ref", "--verify", "--quiet", &remote_ref]).await? {
            (true, _, _) => true,
            (false, _, stderr) if stderr.is_empty() => false,
            (false, _, stderr) => {
                anyhow::bail!(command_error(&stderr, "remote branch inspection failed"))
            }
        };
    let revision = if reference_exists {
        format!("{remote_ref}..HEAD")
    } else {
        "HEAD".to_string()
    };
    match run_git(path, &["rev-list", "--count", &revision]).await? {
        (true, count, _) => count
            .parse()
            .map_err(|_| anyhow::anyhow!("git returned an invalid commit count")),
        (false, _, stderr) => anyhow::bail!(command_error(&stderr, "commit inspection failed")),
    }
}
