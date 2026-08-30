//! Exact Git primitives used by nested sync and update orchestration.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

use crate::git::operations::run_git;
use crate::git::remote::{inspect_remote, policy_violation, RemoteDirection};

fn path_to_str(path: &Path) -> Result<&str> {
    path.to_str()
        .context("Path contains invalid UTF-8 characters")
}

pub(super) fn has_uncommitted_changes(path: &Path) -> Result<bool> {
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

pub(super) fn stash_changes(path: &Path) -> Result<()> {
    let output = Command::new("git")
        .args([
            "-C",
            path_to_str(path)?,
            "stash",
            "push",
            "--include-untracked",
            "-m",
            "repos-subrepo-sync: auto-stash",
        ])
        .output()
        .context("Failed to run git stash")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git stash failed: {stderr}");
    }

    Ok(())
}

pub(super) fn checkout_commit(path: &Path, commit: &str, force: bool) -> Result<()> {
    let mut command = Command::new("git");
    command.args(["-C", path_to_str(path)?, "checkout"]);
    if force {
        command.arg("--force");
    }
    let output = command
        .arg(commit)
        .output()
        .context("Failed to run git checkout")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git checkout failed: {stderr}");
    }

    Ok(())
}

pub(super) fn is_ancestor(path: &Path, ancestor: &str, descendant: &str) -> Result<bool> {
    let output = Command::new("git")
        .args([
            "-C",
            path_to_str(path)?,
            "merge-base",
            "--is-ancestor",
            ancestor,
            descendant,
        ])
        .output()
        .context("Failed to run git merge-base")?;

    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => anyhow::bail!(
            "git merge-base failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    }
}

async fn fetch_origin(path: &Path) -> Result<()> {
    let contexts = inspect_remote(path, "origin", RemoteDirection::Fetch)
        .await
        .context("Failed to inspect nested origin")?;
    if let Some(violation) = policy_violation(&contexts)? {
        anyhow::bail!(violation.message());
    }

    let (success, _, stderr) = run_git(path, &["fetch", "origin"])
        .await
        .context("Failed to run nested git fetch")?;
    if !success {
        anyhow::bail!("git fetch failed: {stderr}");
    }

    Ok(())
}

fn commit_exists(path: &Path, commit: &str) -> Result<bool> {
    let object = format!("{commit}^{{commit}}");
    let output = Command::new("git")
        .args(["-C", path_to_str(path)?, "cat-file", "-e", &object])
        .output()
        .context("Failed to inspect target commit")?;
    Ok(output.status.success())
}

pub(super) async fn ensure_commit_available(path: &Path, commit: &str) -> Result<()> {
    if commit_exists(path, commit)? {
        return Ok(());
    }
    fetch_origin(path).await?;
    if commit_exists(path, commit)? {
        Ok(())
    } else {
        anyhow::bail!(
            "target commit {} is unavailable",
            commit.chars().take(7).collect::<String>()
        )
    }
}

pub(super) async fn fetch_latest_commit(path: &Path) -> Result<String> {
    let path_str = path_to_str(path)?;
    fetch_origin(path).await?;

    for branch in &["origin/HEAD", "origin/main", "origin/master"] {
        let output = Command::new("git")
            .args(["-C", path_str, "rev-parse", branch])
            .output();

        if let Ok(output) = output {
            if output.status.success() {
                return Ok(String::from_utf8(output.stdout)?.trim().to_string());
            }
        }
    }

    anyhow::bail!("Could not determine latest commit from remote")
}
