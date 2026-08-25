//! GitHub repository visibility inspection and process-local caching.

use dashmap::DashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use tokio::process::Command;

use super::run_git;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RepoVisibility {
    Public,
    Private,
    Unknown,
}

static VISIBILITY_CACHE: OnceLock<DashMap<PathBuf, RepoVisibility>> = OnceLock::new();

fn visibility_cache() -> &'static DashMap<PathBuf, RepoVisibility> {
    VISIBILITY_CACHE.get_or_init(DashMap::new)
}

/// Detects repository visibility using the GitHub CLI with in-memory caching.
pub async fn get_repo_visibility(path: &Path) -> RepoVisibility {
    let cache = visibility_cache();
    if let Some(visibility) = cache.get(path) {
        return *visibility;
    }

    let visibility = get_repo_visibility_uncached(path).await;
    cache.insert(path.to_path_buf(), visibility);
    visibility
}

async fn get_repo_visibility_uncached(path: &Path) -> RepoVisibility {
    let remote_url = match run_git(path, &["remote", "get-url", "origin"]).await {
        Ok((true, url, _)) => url,
        _ => return RepoVisibility::Unknown,
    };

    if !crate::git::remote::is_github_remote_url(&remote_url) {
        return RepoVisibility::Unknown;
    }

    let result = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new("gh")
            .args(["repo", "view", "--json", "isPrivate", "-q", ".isPrivate"])
            .current_dir(path)
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) if output.status.success() => {
            match String::from_utf8_lossy(&output.stdout).trim() {
                "true" => RepoVisibility::Private,
                "false" => RepoVisibility::Public,
                _ => RepoVisibility::Unknown,
            }
        }
        _ => RepoVisibility::Unknown,
    }
}
