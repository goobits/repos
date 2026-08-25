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

pub async fn push_lfs_objects(path: &Path, remote: &str, branch: &str) -> (bool, String) {
    match run_git(path, &["lfs", "push", "--all", remote, branch]).await {
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
