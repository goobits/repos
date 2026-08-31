//! Worktree staging, commit, and release-tag operations.

use super::*;

/// Stages tracked modifications and deletions only.
pub async fn stage_tracked_changes(path: &Path) -> Result<(bool, String, String)> {
    run_git(path, &["add", "-u"]).await
}

/// Stages all non-ignored changes, including untracked files.
pub async fn stage_all_changes(path: &Path) -> Result<(bool, String, String)> {
    run_git(path, &["add", "-A"]).await
}

pub async fn stage_files(path: &Path, pattern: &str) -> Result<(bool, String, String)> {
    let mut args = Vec::from(GIT_ADD_ARGS);
    args.push(pattern);
    run_git(path, &args).await
}

pub async fn unstage_files(path: &Path, pattern: &str) -> Result<(bool, String, String)> {
    let mut args = Vec::from(GIT_RESTORE_STAGED_ARGS);
    args.push(pattern);
    run_git(path, &args).await
}

pub async fn get_staging_status(path: &Path) -> Result<(String, String)> {
    match run_git(path, GIT_STATUS_PORCELAIN_ARGS).await {
        Ok((true, stdout, stderr)) => Ok((stdout, stderr)),
        Ok((false, _, stderr)) => Err(anyhow::anyhow!(command_error(
            &stderr,
            "status inspection failed"
        ))),
        Err(error) => Err(error),
    }
}

pub async fn has_staged_changes(path: &Path) -> Result<bool> {
    match run_git(path, GIT_DIFF_CACHED_ARGS).await {
        Ok((success, _, _)) => Ok(!success),
        Err(error) => Err(error),
    }
}

pub async fn commit_changes(
    path: &Path,
    message: &str,
    allow_empty: bool,
) -> Result<(bool, String, String)> {
    let mut args = Vec::from(GIT_COMMIT_ARGS);
    args.push(message);
    if allow_empty {
        args.insert(1, "--allow-empty");
    }
    run_git(path, &args).await
}

/// Checks for tracked, staged, or untracked work using byte-safe porcelain-v2.
pub async fn has_uncommitted_changes(path: &Path) -> Result<bool> {
    let _ = run_git(path, &["update-index", "--refresh"]).await;
    Ok(crate::git::worktree::inspect_worktree(path)
        .await?
        .is_dirty())
}

pub async fn is_detached_head(path: &Path) -> Result<bool> {
    match run_git(path, GIT_REV_PARSE_HEAD_ARGS).await {
        Ok((true, branch, _)) => Ok(branch == DETACHED_HEAD_BRANCH),
        Ok((false, _, stderr)) => Err(anyhow::anyhow!(stderr)),
        Err(error) => Err(error),
    }
}

/// Creates a Git tag and pushes that immutable release tag to the selected remote.
pub async fn create_and_push_tag(path: &Path, tag_name: &str) -> (bool, String) {
    let tag_ref = format!("refs/tags/{tag_name}^{{commit}}");
    let existing_target = run_git(path, &["rev-parse", "--verify", &tag_ref]).await;
    if let Ok((true, existing_target, _)) = &existing_target {
        let head = match run_git(path, &["rev-parse", "HEAD"]).await {
            Ok((true, head, _)) => head,
            Ok((false, _, stderr)) => {
                return (false, format!("failed to resolve release commit: {stderr}"));
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

    let (success, _, stderr) = match run_git(path, &["tag", "--", tag_name]).await {
        Ok(result) => result,
        Err(error) => return (false, format!("failed to create tag: {error}")),
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
                );
            }
            Err(error) => {
                return (
                    false,
                    format!("tag created locally; remote inspection failed: {error}"),
                );
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
    if check_uses_git_lfs(path).await {
        if let Some(error) = super::lfs::option_like_lfs_remote_error(&remote_name) {
            return (false, format!("tag created locally; {error}"));
        }
    }

    let tag_refspec = format!("refs/tags/{tag_name}:refs/tags/{tag_name}");
    match run_git(path, &["push", "--", &remote_name, &tag_refspec]).await {
        Ok((true, _, _)) if existed => (true, format!("existing tag pushed {tag_name}")),
        Ok((true, _, _)) => (true, format!("tagged & pushed {tag_name}")),
        Ok((false, _, stderr)) => (
            false,
            format!(
                "tag exists locally; push failed: {}",
                stderr.lines().next().unwrap_or("unknown error")
            ),
        ),
        Err(error) => (false, format!("tag exists locally; push failed: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::create_and_push_tag;
    use std::path::Path;
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

    #[tokio::test]
    async fn lfs_tag_push_rejects_an_option_like_remote_before_transfer() {
        if !Command::new("git")
            .args(["lfs", "version"])
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return;
        }

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
        git(&repository, &["lfs", "install", "--local"]);
        git(&repository, &["lfs", "track", "*.bin"]);
        std::fs::write(repository.join("asset.bin"), "LFS content").expect("write fixture");
        git(&repository, &["add", ".gitattributes", "asset.bin"]);
        git(&repository, &["commit", "-m", "Initial"]);

        let (success, message) = create_and_push_tag(&repository, "v1.0.0").await;
        assert!(!success);
        assert!(message.contains("rename it without a leading '-'"));
        assert_eq!(git(&repository, &["tag", "--list", "v1.0.0"]), "v1.0.0");

        let refs = Command::new("git")
            .args(["show-ref", "--tags"])
            .current_dir(&remote)
            .output()
            .expect("inspect remote tags");
        assert!(!refs.status.success());
    }
}
