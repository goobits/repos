//! Pull mutation and failed-rebase recovery.

use super::super::{
    inspect_operation_remote, is_rate_limit_error, result_from_fetch_state, run_git, GitFailure,
    GitOperationPhase, GitOperationResult, RemoteDirection, Status, STATUS_SYNCED,
};
use super::{remote_url_fingerprint, validate_pull_snapshot, PullFetchResult};
use std::path::Path;

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

    if let Err(error) = validate_pull_snapshot(path, fetch_result).await {
        return pull_snapshot_changed(fetch_result, &error);
    }

    let Some(upstream_commit) = fetch_result.upstream_commit.as_deref() else {
        return GitOperationResult::new(
            Status::PullError,
            "pull integration requires an inspected upstream commit".to_string(),
            fetch_result.has_uncommitted,
        );
    };
    let Some(fetch_remote) = fetch_result
        .remote
        .as_ref()
        .map(|remote| remote.remote.as_str())
    else {
        return GitOperationResult::new(
            Status::PullError,
            "pull integration requires an inspected fetch remote".to_string(),
            fetch_result.has_uncommitted,
        );
    };
    let (current_remote, policy_failure) =
        match inspect_operation_remote(path, fetch_remote, RemoteDirection::Fetch).await {
            Ok(result) => result,
            Err(error) => {
                let failure = GitFailure::from_message(
                    GitOperationPhase::RemoteInspection,
                    format!("remote inspection failed before LFS fetch: {error}"),
                    fetch_result.remote.clone(),
                );
                return GitOperationResult::failed(
                    Status::PullError,
                    failure,
                    fetch_result.has_uncommitted,
                );
            }
        };
    if let Some(failure) = policy_failure {
        return GitOperationResult::failed(
            Status::PullError,
            failure,
            fetch_result.has_uncommitted,
        );
    }
    if current_remote != fetch_result.remote {
        let error = anyhow::anyhow!("fetch remote changed after pull analysis; rerun the command");
        return pull_snapshot_changed(fetch_result, &error);
    }
    let current_fingerprint = match remote_url_fingerprint(path, fetch_remote).await {
        Ok(fingerprint) => fingerprint,
        Err(error) => return pull_snapshot_changed(fetch_result, &error),
    };
    if fetch_result.remote_fingerprint != Some(current_fingerprint) {
        let error =
            anyhow::anyhow!("fetch remote URL changed after pull analysis; rerun the command");
        return pull_snapshot_changed(fetch_result, &error);
    }
    let uses_lfs = match super::super::lfs::fetch_lfs_for_commit(
        path,
        fetch_remote,
        upstream_commit,
        fetch_result.lfs_endpoint_fingerprint,
    )
    .await
    {
        Ok(uses_lfs) => uses_lfs,
        Err(error) => return pull_lfs_failure(fetch_result, &error),
    };
    if let Err(error) = validate_pull_snapshot(path, fetch_result).await {
        return pull_snapshot_changed(fetch_result, &error);
    }
    // The inspection phase already fetched the selected remote. Integrate the
    // exact upstream snapshot it analyzed instead of letting `git pull` fetch a
    // second time and race with a moving remote.
    let mut integration_args = if uses_lfs {
        vec!["-c", "lfs.fetchinclude=", "-c", "lfs.fetchexclude="]
    } else {
        Vec::new()
    };
    if use_rebase {
        match fetch_result.pre_fetch_upstream_commit.as_deref() {
            Some(previous_upstream) => {
                integration_args.extend(["rebase", "--onto", upstream_commit, previous_upstream]);
            }
            None => integration_args.extend(["rebase", "--", upstream_commit]),
        }
    } else {
        integration_args.extend(["merge", "--ff-only", "--", upstream_commit]);
    }

    match run_git(path, &integration_args).await {
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
            let error_message = recover_failed_rebase(path, use_rebase, &error_message).await;
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
            let error_message = recover_failed_rebase(path, use_rebase, &error_message).await;
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

fn pull_lfs_failure(fetch_result: &PullFetchResult, error: &anyhow::Error) -> GitOperationResult {
    let message = crate::core::clean_error_message(&error.to_string());
    let failure = GitFailure::from_message(GitOperationPhase::Pull, message, None);
    GitOperationResult::failed(Status::PullError, failure, fetch_result.has_uncommitted)
}

fn pull_snapshot_changed(
    fetch_result: &PullFetchResult,
    error: &anyhow::Error,
) -> GitOperationResult {
    let message = crate::core::clean_error_message(&error.to_string());
    let failure = GitFailure::from_message(
        GitOperationPhase::Pull,
        message,
        fetch_result.remote.clone(),
    );
    GitOperationResult::failed(Status::PullError, failure, fetch_result.has_uncommitted)
}

async fn recover_failed_rebase(path: &Path, use_rebase: bool, error_message: &str) -> String {
    if !use_rebase {
        return error_message.to_string();
    }

    let rebase_head = match run_git(path, &["rev-parse", "--verify", "--quiet", "REBASE_HEAD"])
        .await
    {
        Ok((true, _, _)) => true,
        Ok((false, _, _)) => false,
        Err(error) => {
            return format!(
                "rebase failed and its recovery state could not be inspected: {error_message}; {error}"
            );
        }
    };
    if !rebase_head {
        return error_message.to_string();
    }

    match run_git(path, &["rebase", "--abort"]).await {
        Ok((true, _, _)) => {
            format!("rebase conflict; aborted and restored original checkout: {error_message}")
        }
        Ok((false, _, stderr)) => format!(
            "rebase conflict and automatic abort failed: {error_message}; abort failed: {}",
            crate::core::clean_error_message(&stderr)
        ),
        Err(error) => format!(
            "rebase conflict and automatic abort failed: {error_message}; abort failed: {error}"
        ),
    }
}
