//! Pull-side repository inspection and fast-forward mutation.

mod integration;

pub use integration::pull_if_needed;
pub(crate) use integration::pull_if_needed_with_context;

use super::*;
use anyhow::bail;
use sha2::{Digest, Sha256};

#[derive(Clone)]
pub struct PullFetchResult {
    pub has_uncommitted: bool,
    pub ahead_count: u32,
    pub behind_count: u32,
    pub upstream_name: Option<String>,
    analyzed_branch: Option<String>,
    analyzed_head_commit: Option<String>,
    pre_fetch_upstream_commit: Option<String>,
    upstream_commit: Option<String>,
    remote_fingerprint: Option<[u8; 32]>,
    lfs_endpoint_fingerprint: Option<[u8; 32]>,
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
            analyzed_branch: None,
            analyzed_head_commit: None,
            pre_fetch_upstream_commit: None,
            upstream_commit: None,
            remote_fingerprint: None,
            lfs_endpoint_fingerprint: None,
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
            analyzed_branch: None,
            analyzed_head_commit: None,
            pre_fetch_upstream_commit: None,
            upstream_commit: None,
            remote_fingerprint: None,
            lfs_endpoint_fingerprint: None,
            status: Status::Error,
            message: failure.message.clone(),
            failure: Some(failure),
            remote: None,
        }
    }
}

pub async fn fetch_and_analyze_for_pull(path: &Path) -> PullFetchResult {
    use crate::core::clean_error_message;

    let initial_state = match inspect_refreshed_repository_state(path).await {
        Ok(state) => state,
        Err(error) => {
            return PullFetchResult::error(clean_error_message(&error.to_string()), false);
        }
    };
    fetch_and_analyze_for_pull_with_state(path, initial_state).await
}

pub(crate) async fn fetch_and_analyze_for_pull_with_state(
    path: &Path,
    initial_state: crate::git::worktree::WorktreeState,
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
                analyzed_branch: None,
                analyzed_head_commit: None,
                pre_fetch_upstream_commit: None,
                upstream_commit: None,
                remote_fingerprint: None,
                lfs_endpoint_fingerprint: None,
                status: Status::Skip,
                message: STATUS_DETACHED_HEAD.to_string(),
                failure: None,
                remote: None,
            };
        }
        HeadState::Unknown => {
            return PullFetchResult::error(
                "branch inspection returned no HEAD state".to_string(),
                has_uncommitted,
            );
        }
    };
    let upstream_ref = match configured_upstream_ref(path, &current_branch).await {
        Ok(upstream_ref) => upstream_ref,
        Err(error) => {
            return PullFetchResult::error(
                clean_error_message(&error.to_string()),
                has_uncommitted,
            );
        }
    };
    let pre_fetch_upstream_commit = if let Some(upstream_ref) = upstream_ref.as_deref() {
        let revision = format!("{upstream_ref}^{{commit}}");
        match try_resolve_commit(path, &revision, "configured upstream commit").await {
            Ok(commit) => commit,
            Err(error) => {
                return PullFetchResult::error(
                    clean_error_message(&error.to_string()),
                    has_uncommitted,
                );
            }
        }
    } else {
        None
    };

    let remotes = match run_git(path, GIT_REMOTE_ARGS).await {
        Ok((true, output, _)) => output,
        Ok((false, _, stderr)) => {
            return PullFetchResult::error(
                command_error(&stderr, "remote inspection failed"),
                has_uncommitted,
            );
        }
        Err(error) => {
            return PullFetchResult::error(
                clean_error_message(&error.to_string()),
                has_uncommitted,
            );
        }
    };

    if remotes.trim().is_empty() {
        return PullFetchResult {
            has_uncommitted,
            ahead_count: 0,
            behind_count: 0,
            upstream_name: None,
            analyzed_branch: None,
            analyzed_head_commit: None,
            pre_fetch_upstream_commit,
            upstream_commit: None,
            remote_fingerprint: None,
            lfs_endpoint_fingerprint: None,
            status: Status::NoRemote,
            message: STATUS_NO_REMOTE.to_string(),
            failure: None,
            remote: None,
        };
    }

    let fetch_remote = match get_branch_remote_name(path, &current_branch, &remotes).await {
        Ok(remote) => remote,
        Err(error) => {
            return PullFetchResult::error(
                clean_error_message(&error.to_string()),
                has_uncommitted,
            );
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
    let remote_fingerprint = match remote_url_fingerprint(path, &fetch_remote).await {
        Ok(fingerprint) => Some(fingerprint),
        Err(error) => {
            return PullFetchResult::error(
                clean_error_message(&error.to_string()),
                has_uncommitted,
            );
        }
    };

    let fetch_args = ["fetch", "--quiet", "--", fetch_remote.as_str()];
    let fetch_error = match run_git(path, &fetch_args).await {
        Ok((true, _, _)) => None,
        Ok((false, _, stderr)) => Some(command_error(&stderr, "fetch failed")),
        Err(error) => Some(clean_error_message(&error.to_string())),
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
            return PullFetchResult::error(
                clean_error_message(&error.to_string()),
                has_uncommitted,
            );
        }
    };
    if final_state.head() != &HeadState::Branch(current_branch.clone()) {
        return PullFetchResult::error(
            "branch changed while repository state was being inspected".to_string(),
            has_uncommitted,
        );
    }
    let final_upstream_ref = match configured_upstream_ref(path, &current_branch).await {
        Ok(upstream_ref) => upstream_ref,
        Err(error) => {
            return PullFetchResult::error(
                clean_error_message(&error.to_string()),
                has_uncommitted,
            );
        }
    };
    if final_upstream_ref != upstream_ref {
        return PullFetchResult::error(
            "upstream changed while repository state was being inspected".to_string(),
            has_uncommitted,
        );
    }
    let Some(upstream_ref) = final_upstream_ref else {
        return PullFetchResult {
            has_uncommitted,
            ahead_count: 0,
            behind_count: 0,
            upstream_name: None,
            analyzed_branch: Some(current_branch),
            analyzed_head_commit: None,
            pre_fetch_upstream_commit,
            upstream_commit: None,
            remote_fingerprint,
            lfs_endpoint_fingerprint: None,
            status: Status::NoUpstream,
            message: STATUS_NO_UPSTREAM.to_string(),
            failure: None,
            remote: fetch_context,
        };
    };
    let upstream_name = final_state
        .upstream()
        .map_or_else(|| upstream_ref.clone(), str::to_string);

    let analyzed_head_commit = match resolve_commit(path, "HEAD^{commit}", "HEAD commit").await {
        Ok(commit) => commit,
        Err(error) => {
            return PullFetchResult::error(
                clean_error_message(&error.to_string()),
                has_uncommitted,
            );
        }
    };
    let upstream_revision = format!("{upstream_ref}^{{commit}}");
    let upstream_commit =
        match resolve_commit(path, &upstream_revision, "configured upstream commit").await {
            Ok(commit) => commit,
            Err(error) => {
                return PullFetchResult::error(
                    clean_error_message(&error.to_string()),
                    has_uncommitted,
                );
            }
        };
    let counts = match crate::git::ancestry::ahead_behind_between(
        path,
        &analyzed_head_commit,
        &upstream_commit,
    )
    .await
    {
        Ok(counts) => counts,
        Err(error) => {
            return PullFetchResult::error(
                clean_error_message(&error.to_string()),
                has_uncommitted,
            );
        }
    };
    let ahead_count = counts.ahead;
    let behind_count = counts.behind;
    let analyzed_branch = Some(current_branch);
    let analyzed_head_commit = Some(analyzed_head_commit);
    let upstream_commit = Some(upstream_commit);
    let lfs_endpoint_fingerprint = match super::lfs::snapshot_lfs_endpoint(
        path,
        &fetch_remote,
        super::lfs::LfsEndpointOperation::Download,
    )
    .await
    {
        Ok(snapshot) => Some(snapshot.fingerprint),
        Err(error) => {
            return PullFetchResult::error(
                clean_error_message(&error.to_string()),
                has_uncommitted,
            );
        }
    };

    if ahead_count > 0 && behind_count > 0 {
        return PullFetchResult {
            has_uncommitted,
            ahead_count,
            behind_count,
            upstream_name: Some(upstream_name),
            analyzed_branch,
            analyzed_head_commit,
            pre_fetch_upstream_commit,
            upstream_commit,
            remote_fingerprint,
            lfs_endpoint_fingerprint,
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
            upstream_name: Some(upstream_name),
            analyzed_branch,
            analyzed_head_commit,
            pre_fetch_upstream_commit,
            upstream_commit,
            remote_fingerprint,
            lfs_endpoint_fingerprint,
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
            upstream_name: Some(upstream_name),
            analyzed_branch,
            analyzed_head_commit,
            pre_fetch_upstream_commit,
            upstream_commit,
            remote_fingerprint,
            lfs_endpoint_fingerprint,
            status: Status::Synced,
            message: format!("{behind_count} commits behind"),
            failure: None,
            remote: fetch_context,
        }
    }
}

async fn resolve_commit(path: &Path, revision: &str, label: &str) -> Result<String> {
    let (success, stdout, stderr) = run_git(path, &["rev-parse", "--verify", revision]).await?;
    if !success {
        bail!(command_error(
            &stderr,
            &format!("{label} could not be resolved")
        ));
    }
    if !matches!(stdout.len(), 40 | 64) || !stdout.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{label} resolved to an invalid commit identifier");
    }
    Ok(stdout)
}

async fn configured_upstream_ref(path: &Path, branch: &str) -> Result<Option<String>> {
    let branch_ref = format!("refs/heads/{branch}");
    let (success, stdout, stderr) =
        run_git(path, &["for-each-ref", "--format=%(upstream)", &branch_ref]).await?;
    if !success {
        bail!(command_error(
            &stderr,
            "configured upstream ref could not be inspected"
        ));
    }
    if stdout.is_empty() {
        return Ok(None);
    }
    if !stdout.starts_with("refs/") || stdout.lines().count() != 1 {
        bail!("configured upstream resolved to an invalid full ref");
    }
    Ok(Some(stdout))
}

async fn remote_url_fingerprint(path: &Path, remote: &str) -> Result<[u8; 32]> {
    if remote == "." {
        return Ok(Sha256::digest(b"local-dot-remote").into());
    }
    let output = run_git_raw(path, &["remote", "get-url", "--all", "--", remote]).await?;
    if !output.success() {
        bail!(command_error(
            &output.stderr_text(),
            "fetch remote URL could not be inspected"
        ));
    }
    Ok(Sha256::digest(&output.stdout).into())
}

async fn try_resolve_commit(path: &Path, revision: &str, label: &str) -> Result<Option<String>> {
    let (success, stdout, stderr) =
        run_git(path, &["rev-parse", "--verify", "--quiet", revision]).await?;
    if !success {
        if stderr.is_empty() {
            return Ok(None);
        }
        bail!(command_error(
            &stderr,
            &format!("{label} could not be resolved")
        ));
    }
    if !matches!(stdout.len(), 40 | 64) || !stdout.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{label} resolved to an invalid commit identifier");
    }
    Ok(Some(stdout))
}

async fn validate_pull_snapshot(path: &Path, fetch_result: &PullFetchResult) -> Result<()> {
    let state = inspect_refreshed_repository_state(path).await?;
    let Some(analyzed_branch) = fetch_result.analyzed_branch.as_deref() else {
        bail!("pull analysis omitted the local branch");
    };
    if state.head() != &HeadState::Branch(analyzed_branch.to_string()) {
        bail!("branch changed after pull analysis; rerun the command");
    }
    if state.is_dirty() {
        bail!("worktree changed after pull analysis; commit or stash, then rerun the command");
    }
    let Some(analyzed_head_commit) = fetch_result.analyzed_head_commit.as_deref() else {
        bail!("pull analysis omitted the local HEAD commit");
    };
    let current_head = resolve_commit(path, "HEAD^{commit}", "HEAD commit").await?;
    if current_head != analyzed_head_commit {
        bail!("HEAD changed after pull analysis; rerun the command");
    }
    Ok(())
}
