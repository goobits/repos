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

/// How a nested repository checkout is connected to its nearest fleet parent.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum NestedCheckoutKind {
    /// An embedded repository with its own Git directory and no parent gitlink.
    Independent,
    /// A repository recorded by the nearest parent's index as a Git submodule.
    Submodule,
    /// A `.git`-file checkout that is not recorded as a parent gitlink.
    LinkedWorktree,
}

impl NestedCheckoutKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Independent => "independent",
            Self::Submodule => "submodule",
            Self::LinkedWorktree => "worktree",
        }
    }
}

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
    /// Checkout relationship to the nearest discovered fleet parent.
    pub checkout_kind: NestedCheckoutKind,
}

/// A gitlink recorded by a fleet parent whose checkout is not initialized.
#[derive(Debug, Clone)]
pub struct DeclaredSubmodule {
    pub parent_repo: String,
    pub parent_path: PathBuf,
    pub relative_path: String,
    pub target_commit: String,
}

/// Summary of discovered subrepos grouped by remote URL
#[derive(Debug)]
pub struct ValidationReport {
    pub total_nested: usize,
    pub by_remote: HashMap<String, Vec<SubrepoInstance>>,
    pub no_remote: Vec<SubrepoInstance>,
    pub uninitialized_submodules: Vec<DeclaredSubmodule>,
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

    #[must_use]
    pub fn checkout_count(&self, kind: NestedCheckoutKind) -> usize {
        self.by_remote
            .values()
            .flatten()
            .chain(self.no_remote.iter())
            .filter(|instance| instance.checkout_kind == kind)
            .count()
    }
}

/// Read the exact HEAD identity and timestamp from one object snapshot.
fn get_head_metadata(path: &Path) -> Result<(String, i64)> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["show", "-s", "--format=%H%x00%ct", "HEAD"])
        .output()
        .context("Failed to inspect nested repository HEAD")?;

    if !output.status.success() {
        anyhow::bail!(
            "git show failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let stdout = String::from_utf8(output.stdout)?;
    let (commit_hash, timestamp) = stdout
        .trim()
        .split_once('\0')
        .context("Invalid nested repository HEAD metadata")?;
    let commit_timestamp = timestamp
        .parse()
        .context("Invalid nested repository commit timestamp")?;
    Ok((commit_hash.to_string(), commit_timestamp))
}

fn get_current_commit(path: &Path) -> Result<String> {
    Ok(get_head_metadata(path)?.0)
}

/// Get the normalized origin URL, distinguishing a missing origin from an
/// inspection failure.
fn get_remote_url(path: &Path) -> Result<Option<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["remote", "get-url", "origin"])
        .output()
        .context("Failed to run git remote")?;

    if output.status.code() == Some(2) {
        return Ok(None);
    }
    if !output.status.success() {
        anyhow::bail!(
            "git remote inspection failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    let url = String::from_utf8(output.stdout)?.trim().to_string();
    Ok(Some(normalize_remote_url(&url)))
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
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args([
            "status",
            "--porcelain=v2",
            "-z",
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
