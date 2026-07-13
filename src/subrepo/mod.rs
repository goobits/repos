//! Subrepo detection and analysis module.
//!
//! This module provides tools for finding and managing Git repositories nested
//! within other Git repositories (subrepos). It can detect drift between
//! subrepos that share the same remote URL.
//!
//! This is command plumbing for `repos nested`. The data types are public for
//! tests and advanced automation, while the CLI remains the primary supported
//! interface.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub mod status;
pub mod sync;
pub mod validation;

/// Represents a single instance of a nested repository.
#[derive(Debug, Clone)]
pub struct SubrepoInstance {
    /// Name of the parent repository.
    pub parent_repo: String,
    #[allow(dead_code)]
    pub parent_path: PathBuf,
    /// Name of the subrepo (usually the directory name).
    pub subrepo_name: String,
    /// Absolute path to the subrepo.
    pub subrepo_path: PathBuf,
    /// Relative path from parent root.
    #[allow(dead_code)]
    pub relative_path: String,
    /// Full commit hash.
    pub commit_hash: String,
    /// Short 7-character commit hash.
    pub short_hash: String,
    /// Remote origin URL, if available.
    pub remote_url: Option<String>,
    /// Whether there are uncommitted changes in the subrepo.
    pub has_uncommitted: bool,
    /// Unix timestamp of the current commit.
    pub commit_timestamp: i64,
}

/// Summary of discovered subrepos grouped by remote URL
#[derive(Debug)]
pub struct ValidationReport {
    pub total_nested: usize,
    pub by_remote: HashMap<String, Vec<SubrepoInstance>>,
    pub no_remote: Vec<SubrepoInstance>,
}

impl ValidationReport {
    #[must_use]
    pub fn shared_subrepos_count(&self) -> usize {
        self.by_remote
            .iter()
            .filter(|(_, instances)| instances.len() > 1)
            .count()
    }

    #[must_use]
    pub fn unique_remotes(&self) -> usize {
        self.by_remote.len()
    }
}

/// Convert path to string with proper error handling
fn path_to_str(path: &Path) -> Result<&str> {
    path.to_str()
        .context("Path contains invalid UTF-8 characters")
}

/// Get current commit hash for a git repository
fn get_current_commit(path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["-C", path_to_str(path)?, "rev-parse", "HEAD"])
        .output()
        .context("Failed to run git rev-parse")?;

    if !output.status.success() {
        anyhow::bail!("git rev-parse failed");
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

/// Get remote URL for a git repository
fn get_remote_url(path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["-C", path_to_str(path)?, "remote", "get-url", "origin"])
        .output()
        .context("Failed to run git remote")?;

    if !output.status.success() {
        anyhow::bail!("No remote 'origin' found");
    }

    let url = String::from_utf8(output.stdout)?.trim().to_string();
    Ok(normalize_remote_url(&url))
}

/// Normalize remote URLs to group equivalent URLs together
fn normalize_remote_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);

    if let Some((authority, path)) = trimmed.split_once(':') {
        if authority.contains('@') && !authority.contains('/') {
            let host = authority.rsplit('@').next().unwrap_or(authority);
            return remote_key(host, path);
        }
    }

    for scheme in ["https://", "http://", "ssh://", "git://"] {
        if let Some(remote) = trimmed.strip_prefix(scheme) {
            if let Some((authority, path)) = remote.split_once('/') {
                let host = authority.rsplit('@').next().unwrap_or(authority);
                return remote_key(host, path);
            }
        }
    }

    trimmed.to_string()
}

fn remote_key(host: &str, path: &str) -> String {
    let host = host.to_ascii_lowercase();
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if host == "github.com" {
        format!("{host}/{}", path.to_ascii_lowercase())
    } else {
        format!("{host}/{path}")
    }
}

/// Check if repo has uncommitted changes.
///
/// Note: This is a synchronous version for use in the validation module.
/// There's an async version in `git::operations`, but this module requires
/// sync operations.
fn has_uncommitted_changes(path: &Path) -> Result<bool> {
    let path_str = path_to_str(path)?;
    let output = Command::new("git")
        .args([
            "-C",
            path_str,
            "status",
            "--porcelain=v1",
            "--untracked-files=normal",
            "--ignore-submodules=dirty",
        ])
        .output()
        .context("Failed to inspect nested repository status")?;

    if !output.status.success() {
        anyhow::bail!(
            "git status failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(!output.stdout.is_empty())
}

/// Get commit timestamp (Unix epoch seconds)
pub(crate) fn get_commit_timestamp(path: &Path, commit_hash: &str) -> i64 {
    let path_str = match path_to_str(path) {
        Ok(s) => s,
        Err(_) => return 0, // Return epoch 0 for invalid paths
    };

    let output = Command::new("git")
        .args(["-C", path_str, "show", "-s", "--format=%ct", commit_hash])
        .output();

    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or(0),
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_remote_url;

    #[test]
    fn normalizes_equivalent_github_transports() {
        let expected = "github.com/owner/repo";

        assert_eq!(
            normalize_remote_url("git@github.com:Owner/Repo.git"),
            expected
        );
        assert_eq!(
            normalize_remote_url("https://github.com/owner/repo/"),
            expected
        );
        assert_eq!(
            normalize_remote_url("ssh://git@github.com/OWNER/REPO.git"),
            expected
        );
    }

    #[test]
    fn preserves_case_for_case_sensitive_remote_paths() {
        assert_eq!(
            normalize_remote_url("https://git.example.com/Team/Repo.git"),
            "git.example.com/Team/Repo"
        );
    }
}
