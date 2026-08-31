//! Fetch-only remote updates and gitlink publication checks.

use super::*;
use std::collections::HashMap;

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
            );
        }
        Err(error) => {
            return GitOperationResult::new(
                Status::Error,
                clean_error_message(&error.to_string()),
                false,
            );
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
            Ok((context, None)) => fetch_remotes.push((remote, context)),
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
        let fetch_error = match run_git(path, &["fetch", "--quiet", "--", remote]).await {
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
        .with_transferred(updated_refs)
    }
}

/// Verifies that a commit is reachable from a freshly fetched remote branch.
pub(crate) async fn remote_refs_contain_commit(path: &Path, commit: &str) -> Result<bool> {
    let remotes = match run_git(path, GIT_REMOTE_ARGS).await {
        Ok((true, remotes, _)) => remotes,
        Ok((false, _, stderr)) => {
            anyhow::bail!(command_error(&stderr, "remote inspection failed"));
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

        match run_git(path, &["fetch", "--quiet", "--", remote]).await {
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
