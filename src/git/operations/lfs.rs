//! Git LFS detection and publication helpers.

use super::*;

pub async fn check_uses_git_lfs(path: &Path) -> bool {
    match run_git(path, GIT_LFS_ENV_ARGS).await {
        Ok((true, _, _)) => {
            let gitattributes = path.join(".gitattributes");
            if let Ok(content) = tokio::fs::read_to_string(&gitattributes).await {
                if content.contains("filter=lfs") {
                    return true;
                }
            }
            if let Ok((true, files, _)) = run_git(path, &["lfs", "ls-files"]).await {
                return !files.trim().is_empty();
            }
            false
        }
        _ => false,
    }
}

pub(crate) async fn fetch_lfs_for_commit(path: &Path, remote: &str, commit: &str) -> Result<bool> {
    let lfs_available = matches!(run_git(path, GIT_LFS_ENV_ARGS).await, Ok((true, _, _)));
    if !lfs_available {
        let (has_attributes, _, stderr) = run_git(
            path,
            &[
                "grep",
                "-I",
                "-q",
                "-E",
                "filter[[:space:]]*=[[:space:]]*lfs",
                commit,
                "--",
                ":(glob)**/.gitattributes",
            ],
        )
        .await?;
        if has_attributes {
            anyhow::bail!("target commit uses Git LFS, but git-lfs is unavailable");
        }
        if !stderr.is_empty() {
            anyhow::bail!(
                "Git LFS target inspection failed: {}",
                command_error(&stderr, "attributes could not be inspected")
            );
        }
        return Ok(false);
    }

    let (uses_lfs, files, stderr) = run_git(
        path,
        &["lfs", "ls-files", "--include=", "--exclude=", commit],
    )
    .await?;
    if !uses_lfs {
        anyhow::bail!(
            "Git LFS target inspection failed: {}",
            command_error(&stderr, "pointers could not be inspected")
        );
    }
    if files.is_empty() {
        return Ok(false);
    }

    let (fetched, _, stderr) = run_git(
        path,
        &[
            "lfs",
            "fetch",
            "--include=",
            "--exclude=",
            "--",
            remote,
            commit,
        ],
    )
    .await?;
    if !fetched {
        anyhow::bail!(
            "Git LFS target fetch failed: {}",
            command_error(&stderr, "objects could not be fetched")
        );
    }
    let (valid, _, stderr) = run_git(
        path,
        &[
            "-c",
            "lfs.fetchinclude=",
            "-c",
            "lfs.fetchexclude=",
            "lfs",
            "fsck",
            "--objects",
            commit,
        ],
    )
    .await?;
    if !valid {
        anyhow::bail!(
            "Git LFS target verification failed: {}",
            command_error(&stderr, "fetched objects are missing or corrupt")
        );
    }
    Ok(true)
}

pub async fn push_lfs_objects(path: &Path, remote: &str, branch: &str) -> (bool, String) {
    match run_git(path, &["lfs", "push", "--all", "--", remote, branch]).await {
        Ok((true, _, _)) => (true, String::new()),
        Ok((false, _, stderr)) => {
            let message = if stderr.is_empty() {
                "LFS push failed".to_string()
            } else {
                format!("LFS: {}", stderr.lines().next().unwrap_or("push failed"))
            };
            (false, message)
        }
        Err(error) => (false, format!("LFS error: {error}")),
    }
}

pub async fn has_pending_lfs_objects(path: &Path) -> bool {
    if let Ok((true, stdout, _)) = run_git(path, &["lfs", "status", "--porcelain"]).await {
        !stdout.trim().is_empty()
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::fetch_lfs_for_commit;
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

    fn initialize(path: &Path) {
        git(path, &["init"]);
        git(path, &["config", "user.name", "repos test"]);
        git(path, &["config", "user.email", "repos@example.invalid"]);
    }

    #[tokio::test]
    async fn exact_non_lfs_commit_needs_no_fetch() {
        let directory = tempfile::tempdir().expect("temporary repository");
        initialize(directory.path());
        std::fs::write(directory.path().join("tracked.txt"), "content")
            .expect("write tracked file");
        git(directory.path(), &["add", "tracked.txt"]);
        git(directory.path(), &["commit", "-m", "Initial"]);
        let commit = git(directory.path(), &["rev-parse", "HEAD"]);

        assert!(!fetch_lfs_for_commit(directory.path(), "origin", &commit)
            .await
            .expect("inspect exact non-LFS commit"));
    }

    #[tokio::test]
    async fn exact_lfs_commit_fetches_from_the_selected_remote() {
        if !Command::new("git")
            .args(["lfs", "version"])
            .output()
            .is_ok_and(|output| output.status.success())
        {
            return;
        }

        let root = tempfile::tempdir().expect("temporary root");
        let repository = root.path().join("repository");
        let remote = root.path().join("remote.git");
        std::fs::create_dir(&repository).expect("create repository");
        initialize(&repository);
        git(root.path(), &["init", "--bare", remote.to_str().unwrap()]);
        git(&repository, &["lfs", "install", "--local"]);
        git(
            &repository,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&repository, &["lfs", "track", "*.bin"]);
        std::fs::write(repository.join("asset.bin"), b"LFS payload").expect("write LFS fixture");
        git(&repository, &["add", ".gitattributes", "asset.bin"]);
        git(&repository, &["commit", "-m", "Add LFS asset"]);
        let commit = git(&repository, &["rev-parse", "HEAD"]);
        let branch = git(&repository, &["branch", "--show-current"]);
        git(&repository, &["push", "-u", "origin", &branch]);

        assert!(fetch_lfs_for_commit(&repository, "origin", &commit)
            .await
            .expect("fetch exact LFS commit"));
    }
}
