//! Subrepo synchronization operations

use super::{SubrepoInstance, ValidationReport};
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// Convert path to string with proper error handling
fn path_to_str(path: &Path) -> Result<&str> {
    path.to_str()
        .context("Path contains invalid UTF-8 characters")
}

/// Find the remote group identified by a subrepo name.
fn find_instances_by_name(report: &ValidationReport, name: &str) -> Result<Vec<SubrepoInstance>> {
    let mut matching_groups: Vec<_> = report
        .by_remote
        .iter()
        .filter(|(_, instances)| {
            instances
                .iter()
                .any(|instance| instance.subrepo_name == name)
        })
        .collect();
    matching_groups.sort_by_key(|(remote, _)| *remote);

    match matching_groups.as_slice() {
        [] => anyhow::bail!("Subrepo '{name}' not found"),
        [(_, instances)] => Ok((*instances).clone()),
        groups => {
            let remotes = groups
                .iter()
                .map(|(remote, _)| remote.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow::bail!("Subrepo name '{name}' is ambiguous across different remotes: {remotes}")
        }
    }
}

/// Check if a repository has uncommitted changes.
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

/// Stash uncommitted changes in a repository
fn stash_changes(path: &Path) -> Result<()> {
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

/// Checkout a specific commit in a git repository
fn checkout_commit(path: &Path, commit: &str) -> Result<()> {
    let output = Command::new("git")
        .args(["-C", path_to_str(path)?, "checkout", commit])
        .output()
        .context("Failed to run git checkout")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git checkout failed: {stderr}");
    }

    Ok(())
}

/// Returns whether `ancestor` can move to `descendant` without discarding commits.
fn is_ancestor(path: &Path, ancestor: &str, descendant: &str) -> Result<bool> {
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

/// Fetch from remote and determine the latest commit
fn fetch_latest_commit(path: &Path) -> Result<String> {
    let path_str = path_to_str(path)?;

    // Fetch from remote
    let fetch_output = Command::new("git")
        .args(["-C", path_str, "fetch", "origin"])
        .output()
        .context("Failed to run git fetch")?;

    if !fetch_output.status.success() {
        let stderr = String::from_utf8_lossy(&fetch_output.stderr);
        anyhow::bail!("git fetch failed: {stderr}");
    }

    // Try to get latest commit from origin/HEAD, then origin/main, then origin/master
    for branch in &["origin/HEAD", "origin/main", "origin/master"] {
        let output = Command::new("git")
            .args(["-C", path_str, "rev-parse", branch])
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                let commit = String::from_utf8(out.stdout)?.trim().to_string();
                return Ok(commit);
            }
        }
    }

    anyhow::bail!("Could not determine latest commit from remote")
}

/// Sync a subrepo to a specific commit across all parent repositories
pub fn sync_subrepo(name: &str, target_commit: &str, stash: bool, force: bool) -> Result<()> {
    let report = super::validation::validate_subrepos()?;
    sync_subrepo_with_report(name, target_commit, stash, force, &report)
}

/// Sync logic that accepts a report (useful for testing)
pub fn sync_subrepo_with_report(
    name: &str,
    target_commit: &str,
    stash: bool,
    force: bool,
    report: &ValidationReport,
) -> Result<()> {
    let instances = find_instances_by_name(report, name)?;

    let short_commit = target_commit.chars().take(7).collect::<String>();
    println!("\n🔄 Syncing {name} to {short_commit}...\n");

    let mut success_count = 0;
    let mut skip_count = 0;
    let mut error_count = 0;
    let mut stashed_count = 0;

    for instance in &instances {
        let has_changes = has_uncommitted_changes(&instance.subrepo_path)?;

        // Handle uncommitted changes
        if has_changes {
            if stash {
                // Stash changes before syncing
                match stash_changes(&instance.subrepo_path) {
                    Ok(()) => {
                        stashed_count += 1;
                    }
                    Err(e) => {
                        println!("  ❌ {} (stash failed: {})", instance.parent_repo, e);
                        error_count += 1;
                        continue;
                    }
                }
            } else if !force {
                // No stash, no force - skip
                println!(
                    "  ⚠️  {} (uncommitted changes, use --stash or clean the repo first)",
                    instance.parent_repo
                );
                skip_count += 1;
                continue;
            }
            // If force=true, proceed without stashing (will discard changes)
        }

        // Checkout the commit
        match checkout_commit(&instance.subrepo_path, target_commit) {
            Ok(()) => {
                println!("  ✅ {}", instance.parent_repo);
                success_count += 1;
            }
            Err(e) => {
                println!("  ❌ {} ({})", instance.parent_repo, e);
                error_count += 1;
            }
        }
    }

    // Summary
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📊 Sync Summary");
    println!("   ✅ {success_count} synced");
    if stashed_count > 0 {
        println!("   📦 {stashed_count} stashed (changes saved, run 'git stash pop' to restore)");
    }
    if skip_count > 0 {
        println!("   ⚠️  {skip_count} skipped (uncommitted changes)");
    }
    if error_count > 0 {
        println!("   ❌ {error_count} failed");
    }
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    if error_count > 0 {
        anyhow::bail!("{error_count} repositories failed to sync");
    }

    Ok(())
}

/// Update a subrepo to the latest commit from remote
pub fn update_subrepo(name: &str, force: bool) -> Result<()> {
    let report = super::validation::validate_subrepos()?;
    update_subrepo_with_report(name, force, &report)
}

/// Update logic that accepts a report (useful for testing)
pub fn update_subrepo_with_report(
    name: &str,
    force: bool,
    report: &ValidationReport,
) -> Result<()> {
    let instances = find_instances_by_name(report, name)?;

    // Use first instance to determine latest commit
    println!("\n🔍 Fetching latest commit for {name}...");
    let latest = fetch_latest_commit(&instances[0].subrepo_path)?;
    let short_latest = latest.chars().take(7).collect::<String>();
    println!("   Latest commit: {short_latest}\n");

    println!("🔄 Updating {name} to {short_latest}...\n");

    let mut success_count = 0;
    let mut skip_count = 0;
    let mut error_count = 0;

    for instance in &instances {
        // Check if already at latest
        if instance.commit_hash == latest {
            println!("  ✨ {} (already at latest)", instance.parent_repo);
            success_count += 1;
            continue;
        }

        // Check for uncommitted changes
        if !force && has_uncommitted_changes(&instance.subrepo_path)? {
            println!("  ⚠️  {} (uncommitted changes)", instance.parent_repo);
            skip_count += 1;
            continue;
        }

        // Fetch and checkout
        match fetch_latest_commit(&instance.subrepo_path) {
            Ok(commit) => {
                if !force {
                    match is_ancestor(&instance.subrepo_path, &instance.commit_hash, &commit) {
                        Ok(true) => {}
                        Ok(false) => {
                            println!(
                                "  ⚠️  {} (local commits diverge from remote)",
                                instance.parent_repo
                            );
                            skip_count += 1;
                            continue;
                        }
                        Err(e) => {
                            println!(
                                "  ❌ {} (history check failed: {})",
                                instance.parent_repo, e
                            );
                            error_count += 1;
                            continue;
                        }
                    }
                }

                match checkout_commit(&instance.subrepo_path, &commit) {
                    Ok(()) => {
                        let old_short = instance.short_hash.clone();
                        println!(
                            "  ✅ {} ({} → {})",
                            instance.parent_repo, old_short, short_latest
                        );
                        success_count += 1;
                    }
                    Err(e) => {
                        println!("  ❌ {} ({})", instance.parent_repo, e);
                        error_count += 1;
                    }
                }
            }
            Err(e) => {
                println!("  ❌ {} (fetch failed: {})", instance.parent_repo, e);
                error_count += 1;
            }
        }
    }

    // Summary
    println!();
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📊 Update Summary");
    println!("   ✅ {success_count} updated");
    if skip_count > 0 {
        println!("   ⚠️  {skip_count} skipped (manual review required)");
    }
    if error_count > 0 {
        println!("   ❌ {error_count} failed");
    }
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!();

    if error_count > 0 {
        anyhow::bail!("{error_count} repositories failed to update");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::find_instances_by_name;
    use crate::subrepo::{SubrepoInstance, ValidationReport};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn instance(name: &str, remote: &str) -> SubrepoInstance {
        SubrepoInstance {
            parent_repo: "parent".to_string(),
            parent_path: PathBuf::from("parent"),
            subrepo_name: name.to_string(),
            subrepo_path: PathBuf::from("parent/subrepo"),
            relative_path: "subrepo".to_string(),
            commit_hash: "0123456789".to_string(),
            short_hash: "0123456".to_string(),
            remote_url: Some(remote.to_string()),
            has_uncommitted: false,
            commit_timestamp: 0,
        }
    }

    #[test]
    fn rejects_same_name_across_different_remotes() {
        let by_remote = HashMap::from([
            (
                "example.com/team-one/shared".to_string(),
                vec![instance("shared", "example.com/team-one/shared")],
            ),
            (
                "example.com/team-two/shared".to_string(),
                vec![instance("shared", "example.com/team-two/shared")],
            ),
        ]);
        let report = ValidationReport {
            total_nested: 2,
            by_remote,
            no_remote: Vec::new(),
        };

        let error = find_instances_by_name(&report, "shared").unwrap_err();
        assert!(error.to_string().contains("ambiguous"));
    }
}
