//! Subrepo detection and analysis module

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use anyhow::{Result, Context};

pub mod validation;
pub mod status;
pub mod sync;

/// Represents a single instance of a nested repository
#[derive(Debug, Clone)]
pub struct SubrepoInstance {
    pub parent_repo: String,
    #[allow(dead_code)]
    pub parent_path: PathBuf,
    pub subrepo_name: String,
    pub subrepo_path: PathBuf,
    #[allow(dead_code)]
    pub relative_path: String,
    pub commit_hash: String,
    pub short_hash: String,
    pub remote_url: Option<String>,
    pub has_uncommitted: bool,
    pub commit_timestamp: i64,  // Unix timestamp for sorting by date
}

/// Summary of discovered subrepos grouped by remote URL
#[derive(Debug)]
pub struct ValidationReport {
    pub total_nested: usize,
    pub by_remote: HashMap<String, Vec<SubrepoInstance>>,
    pub no_remote: Vec<SubrepoInstance>,
}

impl ValidationReport {
    pub fn shared_subrepos_count(&self) -> usize {
        self.by_remote.iter()
            .filter(|(_, instances)| instances.len() > 1)
            .count()
    }

    pub fn unique_remotes(&self) -> usize {
        self.by_remote.len()
    }
}

/// Get current commit hash for a git repository
fn get_current_commit(path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(&["-C", path.to_str().unwrap(), "rev-parse", "HEAD"])
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
        .args(&["-C", path.to_str().unwrap(), "remote", "get-url", "origin"])
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
    url.trim_end_matches(".git")
        .trim_end_matches('/')
        .replace("git@github.com:", "https://github.com/")
        .to_lowercase()
}

/// Check if repo has uncommitted changes (tracked files only)
///
/// Note: This is a synchronous version for use in the validation module.
/// There's an async version in git::operations, but this module requires
/// sync operations. This only checks tracked files (diff-index vs HEAD).
/// See subrepo/sync.rs for a more conservative version that includes untracked files.
fn has_uncommitted_changes(path: &Path) -> bool {
    let output = Command::new("git")
        .args(&["-C", path.to_str().unwrap(), "diff-index", "--quiet", "HEAD", "--"])
        .output();

    match output {
        Ok(out) => !out.status.success(),
        Err(_) => false,
    }
}

/// Get commit timestamp (Unix epoch seconds)
pub(crate) fn get_commit_timestamp(path: &Path, commit_hash: &str) -> i64 {
    let output = Command::new("git")
        .args(&[
            "-C",
            path.to_str().unwrap(),
            "show",
            "-s",
            "--format=%ct",
            commit_hash,
        ])
        .output();

    match output {
        Ok(out) if out.status.success() => {
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse()
                .unwrap_or(0)
        }
        _ => 0,
    }
}
