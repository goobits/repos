//! Pull-side repository inspection and fast-forward mutation.

use super::*;

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

    let fetch_error = match run_git(path, GIT_FETCH_ARGS).await {
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
            status: Status::Synced,
            message: format!("{behind_count} commits behind"),
            failure: None,
            remote: fetch_context,
        }
    }
}

pub async fn pull_if_needed(
    path: &Path,
    fetch_result: &PullFetchResult,
    use_rebase: bool,
) -> (Status, String, bool) {
    pull_if_needed_with_context(path, fetch_result, use_rebase)
        .await
        .into_tuple()
}

pub(crate) async fn pull_if_needed_with_context(
    path: &Path,
    fetch_result: &PullFetchResult,
    use_rebase: bool,
) -> GitOperationResult {
    use crate::core::clean_error_message;

    let can_rebase_diverged_branch = use_rebase
        && fetch_result.status == Status::PullError
        && fetch_result.ahead_count > 0
        && fetch_result.behind_count > 0;
    if fetch_result.status != Status::Synced && !can_rebase_diverged_branch {
        return result_from_fetch_state(
            fetch_result.status,
            &fetch_result.message,
            fetch_result.has_uncommitted,
            fetch_result.failure.clone(),
        );
    }
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

    let uses_lfs = check_uses_git_lfs(path).await;
    if uses_lfs {
        let _ = run_git(path, &["lfs", "fetch"]).await;
    }
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
            .with_transferred(fetch_result.behind_count.into())
        }
        Ok((false, _, stderr)) => {
            let error_message = clean_error_message(&stderr);
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
        Err(error) => {
            let error_message = clean_error_message(&error.to_string());
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
