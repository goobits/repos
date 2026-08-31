//! Basic git operations and command execution

mod fetch;
mod lfs;
mod pull;
mod push;
mod remotes;
mod visibility;
mod worktree;

pub use fetch::{fetch_and_analyze, FetchResult};
pub use lfs::{check_uses_git_lfs, has_pending_lfs_objects, push_lfs_objects};
pub use pull::{fetch_and_analyze_for_pull, pull_if_needed, PullFetchResult};
pub(crate) use pull::{fetch_and_analyze_for_pull_with_state, pull_if_needed_with_context};
pub use push::push_if_needed;
pub(crate) use push::push_if_needed_with_context;
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
pub(crate) async fn get_upstream_push_target(
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

    default_remote_name(current_branch, remotes, "fetch")
}

async fn get_auto_upstream_remote_name(
    path: &Path,
    current_branch: &str,
    remotes: &str,
) -> Result<String> {
    for key in [
        format!("branch.{current_branch}.pushRemote"),
        "remote.pushDefault".to_string(),
        format!("branch.{current_branch}.remote"),
    ] {
        if let Some(remote) = get_git_config(path, &key).await? {
            return Ok(remote);
        }
    }

    default_remote_name(current_branch, remotes, "push")
}

fn default_remote_name(current_branch: &str, remotes: &str, operation: &str) -> Result<String> {
    let remote_names = remotes
        .lines()
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    if remote_names.contains(&"origin") {
        return Ok("origin".to_string());
    }
    if let [remote] = remote_names.as_slice() {
        return Ok((*remote).to_string());
    }

    let setting = if operation == "push" {
        format!("branch.{current_branch}.pushRemote or remote.pushDefault")
    } else {
        format!("branch.{current_branch}.remote")
    };
    anyhow::bail!("ambiguous {operation} remote; set {setting}")
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
