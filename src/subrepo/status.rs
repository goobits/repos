//! Subrepo status analysis and drift detection

use super::SubrepoInstance;
use anyhow::Result;
use std::collections::{HashMap, HashSet};

/// Uncommitted changes state across instances
#[derive(Debug, PartialEq)]
enum UncommittedState {
    /// All instances are clean (no uncommitted changes)
    AllClean,
    /// All instances have uncommitted changes
    AllDirty,
    /// Some instances are clean, some have uncommitted changes
    Mixed,
}

/// Status of a single subrepo across multiple parent repositories
#[derive(Debug)]
pub struct SubrepoStatus {
    pub name: String,
    pub remote_url: String,
    pub instances: Vec<SubrepoInstance>,
    pub sync_score: f32,
    pub unique_commits: usize,
    pub has_drift: bool,
}

impl SubrepoStatus {
    /// Calculate sync score: (`total_instances` - `unique_commits`) / (`total_instances` - 1) × 100
    ///
    /// Examples:
    /// - 2 instances, same commit   → (2-1)/(2-1) = 100%
    /// - 2 instances, diff commits  → (2-2)/(2-1) = 0%
    /// - 3 instances, 2 commits     → (3-2)/(3-1) = 50%
    fn calculate_sync_score(instances: &[SubrepoInstance]) -> (f32, usize) {
        let unique_commits = instances
            .iter()
            .map(|i| &i.commit_hash)
            .collect::<HashSet<_>>()
            .len();

        if instances.len() <= 1 {
            return (100.0, unique_commits);
        }

        let score =
            ((instances.len() - unique_commits) as f32) / ((instances.len() - 1) as f32) * 100.0;
        (score, unique_commits)
    }

    /// Create a new `SubrepoStatus` from instances
    #[must_use]
    pub fn new(name: String, remote_url: String, instances: Vec<SubrepoInstance>) -> Self {
        let (sync_score, unique_commits) = Self::calculate_sync_score(&instances);
        let has_drift = sync_score < 100.0;

        SubrepoStatus {
            name,
            remote_url,
            instances,
            sync_score,
            unique_commits,
            has_drift,
        }
    }
}

/// Analyze all subrepos and return status for shared ones
pub fn analyze_subrepos() -> Result<Vec<SubrepoStatus>> {
    let report = super::validation::validate_subrepos()?;

    let mut statuses = Vec::new();
    for (remote_url, instances) in report.by_remote {
        // Skip non-shared subrepos
        if instances.len() <= 1 {
            continue;
        }

        let name = instances[0].subrepo_name.clone();
        statuses.push(SubrepoStatus::new(name, remote_url, instances));
    }

    // Sort by sync score (worst first)
    // Use total_cmp to handle NaN safely (treats NaN as less than all other values)
    statuses.sort_by(|a, b| a.sync_score.total_cmp(&b.sync_score));

    Ok(statuses)
}

/// Display concise drift summary for use in repos push
pub fn display_drift_summary(statuses: &[SubrepoStatus]) {
    let drifted: Vec<_> = statuses.iter().filter(|s| s.has_drift).collect();

    if drifted.is_empty() {
        return; // Don't show anything if no drift
    }

    let synced: Vec<_> = statuses.iter().filter(|s| !s.has_drift).collect();

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🔴 SUBREPO DRIFT ({})", drifted.len());
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    for status in &drifted {
        display_drift_summary_item(status);
    }

    if !synced.is_empty() {
        println!("💡 {} subrepos are fully synced.", synced.len());
    }
    println!("💡 Run 'repos subrepo status' for a detailed analysis.");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
}

/// Display a single drifted subrepo in concise format
fn display_drift_summary_item(status: &SubrepoStatus) {
    println!(
        "{}: {} instances at different commits",
        status.name,
        status.instances.len()
    );

    // Find the latest clean commit (sync target)
    let latest_clean = status
        .instances
        .iter()
        .filter(|i| !i.has_uncommitted)
        .max_by_key(|i| i.commit_timestamp);

    // Find absolute latest - instances is guaranteed non-empty by caller
    let Some(latest) = status.instances.iter().max_by_key(|i| i.commit_timestamp) else {
        return; // Defensive: skip if somehow empty
    };

    // Group by commit for display
    let mut by_commit: std::collections::HashMap<String, Vec<&SubrepoInstance>> =
        std::collections::HashMap::new();
    for instance in &status.instances {
        by_commit
            .entry(instance.commit_hash.clone())
            .or_default()
            .push(instance);
    }

    // Sort commits by timestamp (newest first)
    let mut commits: Vec<_> = by_commit.into_iter().collect();
    commits.sort_by(|a, b| {
        let a_timestamp = a.1.iter().map(|i| i.commit_timestamp).max().unwrap_or(0);
        let b_timestamp = b.1.iter().map(|i| i.commit_timestamp).max().unwrap_or(0);
        b_timestamp.cmp(&a_timestamp)
    });

    // Display commits with arrow notation
    for (_commit, instances) in &commits {
        for instance in instances {
            let is_sync_target =
                latest_clean.is_some_and(|t| t.commit_hash == instance.commit_hash);
            let is_latest = latest.commit_hash == instance.commit_hash;

            let prefix = if is_sync_target { "→" } else { " " };
            let status_indicator = if instance.has_uncommitted {
                "⚠️ uncommitted"
            } else {
                "✅ clean"
            };

            let mut suffix = String::new();
            if is_latest && !instance.has_uncommitted {
                suffix.push_str("  ⬆️ LATEST");
            } else if !is_latest {
                suffix.push_str("  (outdated)");
            }

            println!(
                "  {} {}  {:30}  {}{}",
                prefix, instance.short_hash, instance.parent_repo, status_indicator, suffix
            );
        }
    }

    // Show sync command
    let target_commit = latest_clean.map_or(&latest.short_hash, |t| &t.short_hash);
    println!(
        "    Sync: repos subrepo sync {} --to {}",
        status.name, target_commit
    );
    println!();
}

/// Display subrepo status (problem-first by default)
pub fn display_status(statuses: &[SubrepoStatus], show_all: bool) {
    if statuses.is_empty() {
        println!("\n✅ No shared subrepos found.");
        println!("   Run 'repos validate' to see all nested repositories.\n");
        return;
    }

    let drifted: Vec<_> = statuses.iter().filter(|s| s.has_drift).collect();
    let synced: Vec<_> = statuses.iter().filter(|s| !s.has_drift).collect();

    println!("\n🔍 Analyzing {} shared subrepos...\n", statuses.len());

    // Show drifted subrepos
    if !drifted.is_empty() {
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("🔴 SUBREPO DRIFT ({})", drifted.len());
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

        for status in &drifted {
            display_drift_status(status);
        }
    }

    // Show synced subrepos if requested
    if show_all && !synced.is_empty() {
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("🟢 SYNCED SUBREPOS ({})", synced.len());
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

        for status in synced {
            display_synced_status(status);
        }
    } else if !synced.is_empty() {
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!("💡 {} subrepos fully synced (100%)", synced.len());
        println!("   Use --all to see them");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    }

    // Add footer with global update suggestion if there's drift
    if !drifted.is_empty() {
        println!();
        println!("🔧 To update a drifted repo to its 'origin/main' branch instead, run:");
        println!("   repos subrepo update <name>  (e.g., 'repos subrepo update docs-engine')");
    }

    println!();
}

/// Analyze the uncommitted state across instances
fn analyze_uncommitted_state(instances: &[SubrepoInstance]) -> UncommittedState {
    let uncommitted_count = instances.iter().filter(|i| i.has_uncommitted).count();

    if uncommitted_count == 0 {
        UncommittedState::AllClean
    } else if uncommitted_count == instances.len() {
        UncommittedState::AllDirty
    } else {
        UncommittedState::Mixed
    }
}

/// Display a drifted subrepo with safe, actionable commands
fn display_drift_status(status: &SubrepoStatus) {
    println!("{}", status.name);
    println!("  Remote: {}", status.remote_url);
    println!(
        "  Sync Score: {}% ({} commits across {} repos)",
        status.sync_score as u32,
        status.unique_commits,
        status.instances.len()
    );
    println!();

    // Find the latest timestamp (absolute latest)
    let latest_timestamp = status
        .instances
        .iter()
        .map(|i| i.commit_timestamp)
        .max()
        .unwrap_or(0);

    // Find the latest CLEAN commit timestamp (sync target)
    let latest_clean_timestamp = status
        .instances
        .iter()
        .filter(|i| !i.has_uncommitted)
        .map(|i| i.commit_timestamp)
        .max();

    // Group instances by commit
    let mut by_commit: HashMap<String, Vec<&SubrepoInstance>> = HashMap::new();
    for instance in &status.instances {
        by_commit
            .entry(instance.commit_hash.clone())
            .or_default()
            .push(instance);
    }

    // Sort commits by number of instances (most common first), then by timestamp (newest first)
    let mut commits: Vec<_> = by_commit.into_iter().collect();
    commits.sort_by(|a, b| {
        // First sort by count (descending)
        let count_cmp = b.1.len().cmp(&a.1.len());
        if count_cmp != std::cmp::Ordering::Equal {
            return count_cmp;
        }
        // Then by timestamp (descending - newest first)
        let a_timestamp = a.1.iter().map(|i| i.commit_timestamp).max().unwrap_or(0);
        let b_timestamp = b.1.iter().map(|i| i.commit_timestamp).max().unwrap_or(0);
        b_timestamp.cmp(&a_timestamp)
    });

    // Display commits and their instances with arrow notation
    for (_commit, instances) in &commits {
        for instance in instances {
            let is_sync_target = latest_clean_timestamp
                .is_some_and(|t| instance.commit_timestamp == t && !instance.has_uncommitted);
            let is_latest = instance.commit_timestamp == latest_timestamp;

            let prefix = if is_sync_target { "→" } else { " " };
            let status_indicator = if instance.has_uncommitted {
                "⚠️ uncommitted"
            } else {
                "✅ clean"
            };

            let mut suffix = String::new();
            if is_latest && !instance.has_uncommitted {
                suffix.push_str("  ⬆️ LATEST");
            } else if !is_latest {
                suffix.push_str("  (outdated)");
            }

            // Align the repo name and status
            println!(
                "  {} {}  {:width$}  {}{}",
                prefix,
                instance.short_hash,
                instance.parent_repo,
                status_indicator,
                suffix,
                width = 30
            );
        }
    }
    println!();

    // Analyze uncommitted state and provide safe, actionable suggestions
    let uncommitted_state = analyze_uncommitted_state(&status.instances);

    match uncommitted_state {
        UncommittedState::AllDirty => {
            println!("  ⚠️ All instances have uncommitted changes.");
            println!();

            // No clean target - use the latest commit (guaranteed non-empty by caller)
            let Some(target_instance) = status.instances.iter().max_by_key(|i| i.commit_timestamp)
            else {
                return; // Defensive: skip if somehow empty
            };
            let target_commit = &target_instance.short_hash;

            let dirty_repos: Vec<&str> = status
                .instances
                .iter()
                .map(|i| i.parent_repo.as_str())
                .collect();
            let repos_desc = if dirty_repos.len() == 1 {
                format!("'{}'", dirty_repos[0])
            } else {
                format!("{} repos", dirty_repos.len())
            };

            println!("  💡 EASY FIX (Recommended):");
            println!(
                "     repos subrepo sync {} --to {} --stash",
                status.name, target_commit
            );
            println!("     (Stashes uncommitted changes in {repos_desc})");
            println!();
            println!("  🔥 FORCE FIX (Discards all local changes):");
            println!(
                "     repos subrepo sync {} --to {} --force",
                status.name, target_commit
            );
        }

        UncommittedState::Mixed => {
            // Find the latest CLEAN commit (the sync target)
            let Some(target_instance) = status
                .instances
                .iter()
                .filter(|i| !i.has_uncommitted)
                .max_by_key(|i| i.commit_timestamp)
            else {
                return; // Defensive: skip if no clean instances
            };
            let target_commit = &target_instance.short_hash;
            let target_repo = &target_instance.parent_repo;

            // Collect dirty repo names
            let dirty_repos: Vec<&str> = status
                .instances
                .iter()
                .filter(|i| i.has_uncommitted)
                .map(|i| i.parent_repo.as_str())
                .collect();

            let dirty_list = if dirty_repos.len() == 1 {
                format!("'{}'", dirty_repos[0])
            } else {
                dirty_repos
                    .iter()
                    .map(|r| format!("'{r}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };

            println!("  💡 EASY FIX (Recommended):");
            println!(
                "     repos subrepo sync {} --to {} --stash",
                status.name, target_commit
            );
            println!("     (Syncs {dirty_list} to the clean commit from '{target_repo}')");
            println!();

            println!("  🔥 FORCE FIX (Discards changes in {dirty_list}):");
            println!(
                "     repos subrepo sync {} --to {} --force",
                status.name, target_commit
            );
        }

        UncommittedState::AllClean => {
            // All clean - normal sync suggestions work
            // Find the latest commit (guaranteed non-empty by caller)
            let Some(latest_commit) = status.instances.iter().max_by_key(|i| i.commit_timestamp)
            else {
                return; // Defensive: skip if somehow empty
            };

            println!("  🔧 SYNC to latest commit:");
            println!(
                "     repos subrepo sync {} --to {}",
                status.name, latest_commit.short_hash
            );
        }
    }

    println!();
}

/// Display a synced subrepo
fn display_synced_status(status: &SubrepoStatus) {
    println!("{}", status.name);
    println!("  Remote: {}", status.remote_url);
    println!("  Sync Score: 100% (all at same commit)");
    println!();

    // Show the commit they're all at
    println!("  {}  (all instances)", status.instances[0].short_hash);
    println!();

    // Check if any have uncommitted changes
    let has_any_uncommitted = status.instances.iter().any(|i| i.has_uncommitted);

    if has_any_uncommitted {
        for instance in &status.instances {
            let status_indicator = if instance.has_uncommitted {
                "⚠️  uncommitted"
            } else {
                "✅ clean"
            };
            println!(
                "    {:width$}  {}",
                instance.parent_repo,
                status_indicator,
                width = 30
            );
        }
        println!();
        println!("  ✅ Already synchronized");
        println!("  ⚠️  But some have uncommitted changes");
    } else {
        for instance in &status.instances {
            println!("    • {}", instance.parent_repo);
        }
        println!();
        println!("  ✅ Already synchronized and clean");
    }

    println!();
}
