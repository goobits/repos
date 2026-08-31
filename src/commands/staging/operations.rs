//! Single-repository staging, unstaging, and commit operations.

use super::*;

fn commit_output_prefix(stdout: &str) -> String {
    stdout
        .lines()
        .next()
        .filter(|line| line.chars().count() > 7)
        .map(|line| line.chars().take(7).collect())
        .unwrap_or_else(|| "committed".to_string())
}

pub(super) async fn perform_staging_operation(
    repo_path: &std::path::Path,
    pattern: &str,
) -> (Status, String) {
    match stage_files(repo_path, pattern).await {
        Ok((true, _, _)) => (Status::Staged, format!("staged {pattern}")),
        Ok((false, _, stderr)) => {
            if stderr.contains("pathspec") && stderr.contains("did not match") {
                (Status::NoChanges, format!("no files match {pattern}"))
            } else {
                (Status::StagingError, clean_error_message(&stderr))
            }
        }
        Err(error) => (
            Status::StagingError,
            clean_error_message(&error.to_string()),
        ),
    }
}

pub(super) async fn perform_commit_operation(
    repo_path: &std::path::Path,
    message: &str,
    include_empty: bool,
    committed_gitlinks: &[PathBuf],
    gitlink_inspection_error: Option<&str>,
) -> (Status, String) {
    if let Some(error) = gitlink_inspection_error {
        return (
            Status::CommitError,
            format!("submodule relationship inspection failed: {error}"),
        );
    }

    match is_detached_head(repo_path).await {
        Ok(true) => {
            return (
                Status::Skip,
                "detached HEAD; checkout a branch before commit".to_string(),
            );
        }
        Ok(false) => {}
        Err(error) => {
            return (
                Status::CommitError,
                format!(
                    "branch check failed: {}",
                    clean_error_message(&error.to_string())
                ),
            );
        }
    }

    for child_path in committed_gitlinks {
        let Ok(relative) = child_path.strip_prefix(repo_path) else {
            return (
                Status::CommitError,
                format!(
                    "failed to refresh submodule pointer outside parent: {}",
                    child_path.display()
                ),
            );
        };
        let Some(relative) = relative.to_str() else {
            return (
                Status::CommitError,
                "failed to refresh non-UTF-8 submodule path".to_string(),
            );
        };
        match crate::git::operations::run_git(repo_path, &["add", "--", relative]).await {
            Ok((true, _, _)) => {}
            Ok((false, _, stderr)) => {
                return (
                    Status::CommitError,
                    format!(
                        "failed to refresh submodule pointer: {}",
                        clean_error_message(&stderr)
                    ),
                );
            }
            Err(error) => {
                return (
                    Status::CommitError,
                    format!("failed to refresh submodule pointer: {error}"),
                );
            }
        }
    }

    if !include_empty {
        match has_staged_changes(repo_path).await {
            Ok(false) => return (Status::NoChanges, "no staged changes".to_string()),
            Ok(true) => {}
            Err(error) => {
                let error_message = clean_error_message(&error.to_string());
                return (
                    Status::CommitError,
                    format!("error checking changes: {error_message}"),
                );
            }
        }
    }

    match commit_changes(repo_path, message, include_empty).await {
        Ok((true, stdout, _)) => {
            let commit_info = commit_output_prefix(&stdout);
            (Status::Committed, format!("committed {commit_info}"))
        }
        Ok((false, _, stderr)) => {
            let error_message = clean_error_message(&stderr);
            if error_message.contains("nothing to commit")
                || error_message.contains("no changes added")
            {
                (Status::NoChanges, "nothing to commit".to_string())
            } else {
                (Status::CommitError, error_message)
            }
        }
        Err(error) => (Status::CommitError, clean_error_message(&error.to_string())),
    }
}

pub(super) async fn perform_unstaging_operation(
    repo_path: &std::path::Path,
    pattern: &str,
) -> (Status, String) {
    match unstage_files(repo_path, pattern).await {
        Ok((true, _, _)) => (Status::Unstaged, format!("unstaged {pattern}")),
        Ok((false, _, stderr)) => {
            if stderr.contains("pathspec") && stderr.contains("did not match") {
                (
                    Status::NoChanges,
                    format!("no staged files match {pattern}"),
                )
            } else {
                (Status::StagingError, clean_error_message(&stderr))
            }
        }
        Err(error) => (
            Status::StagingError,
            clean_error_message(&error.to_string()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::commit_output_prefix;

    #[test]
    fn commit_output_prefix_is_unicode_safe() {
        assert_eq!(
            commit_output_prefix("[abc💥branch deadbee] message"),
            "[abc💥br"
        );
    }

    #[test]
    fn commit_output_prefix_handles_short_or_empty_output() {
        assert_eq!(commit_output_prefix("short"), "committed");
        assert_eq!(commit_output_prefix(""), "committed");
    }
}
