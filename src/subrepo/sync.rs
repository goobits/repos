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

/// Find all instances of a subrepo by name
///
/// Note: If multiple different remotes have subrepos with the same name,
/// this will return ALL instances across all remotes. This is intentional
/// to avoid confusion when syncing.
fn find_instances_by_name(report: &ValidationReport, name: &str) -> Result<Vec<SubrepoInstance>> {
    let mut instances = Vec::new();

    // Collect ALL instances with matching name, even from different remotes
    for instance_list in report.by_remote.values() {
        if let Some(first) = instance_list.first() {
            if first.subrepo_name == name {
                instances.extend(instance_list.clone());
                // Don't break - continue checking other remotes
            }
        }
    }

    if instances.is_empty() {
        anyhow::bail!(
            "Subrepo '{name}' not found or not shared across multiple repos"
        );
    }

    Ok(instances)
}

/// Check if a repository has uncommitted changes (including untracked files)
///
/// Note: This is more conservative than the versions in mod.rs and `git::operations`,
/// which only check tracked files (diff-index). For sync operations, we want to be
/// extra cautious and block syncing if there are ANY changes, including untracked files.
/// Uses `git status --porcelain` to detect all modifications.
fn has_uncommitted_changes(path: &Path) -> bool {
    let path_str = match path_to_str(path) {
        Ok(s) => s,
        Err(_) => return false, // Treat invalid paths as no changes
    };

    let output = Command::new("git")
        .args(["-C", path_str, "status", "--porcelain"])
        .output();

    match output {
        Ok(out) => !out.stdout.is_empty(),
        Err(_) => false,
    }
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
pub async fn sync_subrepo(name: &str, target_commit: &str, stash: bool, force: bool) -> Result<()> {
    let report = super::validation::validate_subrepos().await?;
    sync_subrepo_with_report(name, target_commit, stash, force, &report)
}

/// Sync logic that accepts a report (useful for testing)
pub fn sync_subrepo_with_report(
    name: &str,
    target_commit: &str,
    stash: bool,
    force: bool,
    report: &ValidationReport
) -> Result<()> {
    let instances = find_instances_by_name(report, name)?;

    let short_commit = target_commit.chars().take(7).collect::<String>();
    println!("\n🔄 Syncing {name} to {short_commit}...\n");

    let mut success_count = 0;
    let mut skip_count = 0;
    let mut error_count = 0;
    let mut stashed_count = 0;

    for instance in &instances {
        let has_changes = has_uncommitted_changes(&instance.subrepo_path);

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
                    "  ⚠️  {} (uncommitted changes, use --stash or --force)",
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
        println!(
            "   📦 {stashed_count} stashed (changes saved, run 'git stash pop' to restore)"
        );
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
pub async fn update_subrepo(name: &str, force: bool) -> Result<()> {
    let report = super::validation::validate_subrepos().await?;
    update_subrepo_with_report(name, force, &report)
}

/// Update logic that accepts a report (useful for testing)
pub fn update_subrepo_with_report(
    name: &str,
    force: bool,
    report: &ValidationReport
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
        if !force && has_uncommitted_changes(&instance.subrepo_path) {
            println!(
                "  ⚠️  {} (uncommitted changes, use --force)",
                instance.parent_repo
            );
            skip_count += 1;
            continue;
        }

        // Fetch and checkout
        match fetch_latest_commit(&instance.subrepo_path) {
            Ok(commit) => match checkout_commit(&instance.subrepo_path, &commit) {
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
            },
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
        println!("   ⚠️  {skip_count} skipped (uncommitted changes)");
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