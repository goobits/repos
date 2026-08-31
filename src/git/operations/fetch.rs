//! Push-side fetch snapshot and upstream analysis.

use super::*;

/// Result of the fetch phase for a repository.
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

/// Fetch and analyze repository state without mutating the worktree.
pub async fn fetch_and_analyze(path: &Path, auto_upstream: bool) -> FetchResult {
    use crate::core::clean_error_message;

    let initial_state = match inspect_refreshed_repository_state(path).await {
        Ok(state) => state,
        Err(error) => {
            return FetchResult::error(
                clean_error_message(&error.to_string()),
                false,
                String::new(),
            )
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

    let remotes = match run_git(path, GIT_REMOTE_ARGS).await {
        Ok((true, output, _)) => output,
        Ok((false, _, stderr)) => {
            return FetchResult::error(
                command_error(&stderr, "remote inspection failed"),
                has_uncommitted,
                String::new(),
            )
        }
        Err(error) => {
            return FetchResult::error(
                clean_error_message(&error.to_string()),
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

    let remote_result = if auto_upstream && initial_state.upstream().is_none() {
        get_auto_upstream_remote_name(path, &current_branch, &remotes).await
    } else {
        get_branch_remote_name(path, &current_branch, &remotes).await
    };
    let fetch_remote = match remote_result {
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
        return FetchResult {
            has_uncommitted,
            current_branch,
            ahead_count: 0,
            behind_count: 0,
            upstream_exists: false,
            upstream_remote: Some(fetch_remote),
            upstream_branch: None,
            status: Status::NoUpstream,
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
            Err(error) => {
                return FetchResult::error(
                    clean_error_message(&error.to_string()),
                    has_uncommitted,
                    current_branch,
                )
            }
        },
    };

    let ahead_count = counts.ahead;
    let behind_count = counts.behind;
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
            status: Status::Synced,
            message: format!("{ahead_count} commits ahead"),
            failure: None,
        }
    }
}
