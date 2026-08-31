//! Push-side mutation after the repository fetch snapshot is complete.

use super::*;

/// Phase 2: Push repository if needed (write operation, moderate concurrency).
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

    if fetch_result.status != Status::Synced && fetch_result.status != Status::NoUpstream {
        return result_from_fetch_state(
            fetch_result.status,
            &fetch_result.message,
            fetch_result.has_uncommitted,
            fetch_result.failure.clone(),
        );
    }

    if fetch_result.upstream_exists && fetch_result.ahead_count == 0 {
        return GitOperationResult::new(
            Status::Synced,
            STATUS_SYNCED.to_string(),
            fetch_result.has_uncommitted,
        );
    }

    // A normal push without an upstream is intentionally a no-op. Decide that
    // before inspecting the push transport or attempting an LFS transfer.
    if !fetch_result.upstream_exists && !auto_upstream {
        return GitOperationResult::new(
            Status::NoUpstream,
            STATUS_NO_UPSTREAM.to_string(),
            fetch_result.has_uncommitted,
        );
    }

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

    let uses_lfs = check_uses_git_lfs(path).await;
    if uses_lfs && has_pending_lfs_objects(path).await {
        let branch = lfs_source_ref(&fetch_result.current_branch);

        let (lfs_success, lfs_error) = push_lfs_objects(path, &remote_name, branch).await;
        if !lfs_success {
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

    if !fetch_result.upstream_exists {
        debug_assert!(auto_upstream);
        let transferred = match count_new_upstream_commits(path, &remote_name, &target_branch).await
        {
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
        let push_args = vec![
            "push",
            "-u",
            "--",
            &remote_name,
            &fetch_result.current_branch,
        ];
        return match run_git(path, &push_args).await {
            Ok((true, _, _)) => {
                let msg = if uses_lfs {
                    format!("set upstream ({remote_name}) & pushed (with LFS)")
                } else {
                    format!("set upstream ({remote_name}) & pushed")
                };
                GitOperationResult::new(Status::Pushed, msg, fetch_result.has_uncommitted)
                    .with_transferred(transferred)
            }
            Ok((false, _, stderr)) => {
                let error_message = clean_error_message(&stderr);
                let failure =
                    GitFailure::from_message(GitOperationPhase::Push, error_message, push_context);
                GitOperationResult::failed(Status::Error, failure, fetch_result.has_uncommitted)
            }
            Err(error) => {
                let error_message = clean_error_message(&error.to_string());
                let failure =
                    GitFailure::from_message(GitOperationPhase::Push, error_message, push_context);
                GitOperationResult::failed(Status::Error, failure, fetch_result.has_uncommitted)
            }
        };
    }

    if fetch_result.ahead_count == 0 {
        return GitOperationResult::new(
            Status::Synced,
            STATUS_SYNCED.to_string(),
            fetch_result.has_uncommitted,
        );
    }

    let push_refspec = if target_branch == fetch_result.current_branch {
        fetch_result.current_branch.clone()
    } else {
        format!("{}:{}", fetch_result.current_branch, target_branch)
    };
    let push_args = vec!["push", "--", &remote_name, &push_refspec];
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
        Err(error) => {
            let error_message = clean_error_message(&error.to_string());
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

fn lfs_source_ref(current_branch: &str) -> &str {
    if current_branch.is_empty() {
        "HEAD"
    } else {
        current_branch
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(path: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn no_upstream_fetch_result() -> FetchResult {
        FetchResult {
            has_uncommitted: false,
            current_branch: "local-feature".to_string(),
            ahead_count: 0,
            behind_count: 0,
            upstream_exists: false,
            upstream_remote: Some("missing-remote".to_string()),
            upstream_branch: None,
            status: Status::NoUpstream,
            message: STATUS_NO_UPSTREAM.to_string(),
            failure: None,
        }
    }

    #[tokio::test]
    async fn no_upstream_without_opt_in_returns_before_remote_inspection() {
        let result = push_if_needed_with_context(
            Path::new("/nonexistent/no-upstream-repository"),
            &no_upstream_fetch_result(),
            false,
        )
        .await;

        assert_eq!(result.status, Status::NoUpstream);
        assert_eq!(result.message, STATUS_NO_UPSTREAM);
    }

    #[test]
    fn lfs_push_uses_the_local_source_ref() {
        assert_eq!(lfs_source_ref("local-feature"), "local-feature");
        assert_eq!(lfs_source_ref(""), "HEAD");
    }

    #[tokio::test]
    async fn fetch_and_push_treat_an_option_like_remote_name_literally() {
        let directory = tempfile::tempdir().expect("temporary root");
        let repository = directory.path().join("repository");
        let remote = directory.path().join("remote.git");
        std::fs::create_dir(&repository).expect("create repository");
        git(&repository, &["init"]);
        git(&repository, &["config", "user.name", "repos test"]);
        git(
            &repository,
            &["config", "user.email", "repos@example.invalid"],
        );
        git(
            directory.path(),
            &["init", "--bare", remote.to_str().unwrap()],
        );
        git(
            &repository,
            &["remote", "add", "--", "--all", remote.to_str().unwrap()],
        );
        std::fs::write(repository.join("tracked.txt"), "initial").expect("write fixture");
        git(&repository, &["add", "tracked.txt"]);
        git(&repository, &["commit", "-m", "Initial"]);

        let first_fetch = fetch_and_analyze(&repository, true).await;
        assert_eq!(first_fetch.status, Status::NoUpstream);
        let first_push = push_if_needed_with_context(&repository, &first_fetch, true).await;
        assert_eq!(first_push.status, Status::Pushed, "{}", first_push.message);

        std::fs::write(repository.join("tracked.txt"), "updated").expect("update fixture");
        git(&repository, &["add", "tracked.txt"]);
        git(&repository, &["commit", "-m", "Update"]);
        let second_fetch = fetch_and_analyze(&repository, false).await;
        assert_eq!(second_fetch.status, Status::Synced);
        assert_eq!(second_fetch.ahead_count, 1);
        let second_push = push_if_needed_with_context(&repository, &second_fetch, false).await;
        assert_eq!(
            second_push.status,
            Status::Pushed,
            "{}",
            second_push.message
        );
    }
}
