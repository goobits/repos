//! Subrepo synchronization operations

mod git;
mod report;

use super::{get_current_commit, SubrepoInstance, ValidationReport};
use crate::core::{clean_error_message, format_relative_repo_path};
use anyhow::Result;
use git::{
    checkout_commit, ensure_commit_available, fetch_latest_commit, has_uncommitted_changes,
    is_ancestor, stash_changes,
};
use report::{
    finish_operation, record_aborted_candidates, NestedOperation, NestedOutcome, NestedOutcomeKind,
};

#[cfg(test)]
use report::generate_operation_report;

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

    let mut outcomes = Vec::with_capacity(instances.len());
    let mut candidates = Vec::new();

    // Inspect every copy and make the immutable target available before any
    // worktree is stashed or moved.
    for instance in &instances {
        let has_changes = match has_uncommitted_changes(&instance.subrepo_path) {
            Ok(has_changes) => has_changes,
            Err(error) => {
                let error = clean_error_message(&error.to_string());
                println!("  ❌ {} (status failed: {})", instance.parent_repo, error);
                outcomes.push(NestedOutcome::new(
                    instance,
                    NestedOutcomeKind::Failed,
                    format!("status failed: {error}"),
                    Some("inspect this nested repository, then retry".to_string()),
                ));
                continue;
            }
        };
        if has_changes && !stash && !force {
            println!(
                "  ⚠️  {} (uncommitted changes, use --stash or clean the repo first)",
                instance.parent_repo
            );
            outcomes.push(NestedOutcome::new(
                instance,
                NestedOutcomeKind::Skipped,
                "uncommitted changes",
                Some(format!(
                    "run `repos nested sync {name} --to {short_commit} --stash` or clean it"
                )),
            ));
            continue;
        }
        if let Err(error) = ensure_commit_available(&instance.subrepo_path, target_commit) {
            let error = clean_error_message(&error.to_string());
            println!(
                "  ❌ {} (target preflight failed: {})",
                instance.parent_repo, error
            );
            outcomes.push(NestedOutcome::new(
                instance,
                NestedOutcomeKind::Failed,
                format!("target preflight failed: {error}"),
                Some("check the nested remote and target commit, then retry".to_string()),
            ));
            continue;
        }
        candidates.push((instance, has_changes));
    }

    if outcomes
        .iter()
        .any(|outcome| outcome.kind == NestedOutcomeKind::Failed)
    {
        record_aborted_candidates(
            candidates.iter().map(|(instance, _)| *instance),
            &mut outcomes,
        );
        return finish_operation(NestedOperation::Sync, &outcomes);
    }

    for (instance, has_changes) in candidates {
        let mut stashed = false;
        if has_changes && stash {
            match stash_changes(&instance.subrepo_path) {
                Ok(()) => stashed = true,
                Err(error) => {
                    let error = clean_error_message(&error.to_string());
                    println!("  ❌ {} (stash failed: {})", instance.parent_repo, error);
                    outcomes.push(NestedOutcome::new(
                        instance,
                        NestedOutcomeKind::Failed,
                        format!("stash failed: {error}"),
                        Some("resolve the stash failure, then retry".to_string()),
                    ));
                    continue;
                }
            }
        }

        match checkout_commit(&instance.subrepo_path, target_commit, force) {
            Ok(()) => {
                println!("  ✅ {}", instance.parent_repo);
                let (message, next) = if stashed {
                    (
                        format!("checked out {short_commit}; local changes stashed"),
                        Some(format!(
                            "run `git -C '{}' stash pop` when ready to restore them",
                            format_relative_repo_path(&instance.subrepo_path.to_string_lossy())
                        )),
                    )
                } else {
                    (format!("checked out {short_commit}"), None)
                };
                outcomes.push(NestedOutcome::new(
                    instance,
                    NestedOutcomeKind::Changed,
                    message,
                    next,
                ));
            }
            Err(e) => {
                let error = clean_error_message(&e.to_string());
                println!("  ❌ {} ({})", instance.parent_repo, error);
                outcomes.push(NestedOutcome::new(
                    instance,
                    NestedOutcomeKind::Failed,
                    error,
                    Some("resolve the checkout failure, then retry".to_string()),
                ));
            }
        }
    }

    finish_operation(NestedOperation::Sync, &outcomes)
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

    let mut outcomes = Vec::with_capacity(instances.len());
    let mut candidates = Vec::new();

    // Preflight every copy against the same commit before changing any checkout.
    for instance in &instances {
        let current_commit = match get_current_commit(&instance.subrepo_path) {
            Ok(commit) => commit,
            Err(error) => {
                let error = clean_error_message(&error.to_string());
                println!(
                    "  ❌ {} (HEAD check failed: {})",
                    instance.parent_repo, error
                );
                outcomes.push(NestedOutcome::new(
                    instance,
                    NestedOutcomeKind::Failed,
                    format!("HEAD check failed: {error}"),
                    Some("inspect this nested repository, then retry".to_string()),
                ));
                continue;
            }
        };

        if current_commit == latest {
            println!("  ✨ {} (already at latest)", instance.parent_repo);
            outcomes.push(NestedOutcome::new(
                instance,
                NestedOutcomeKind::Unchanged,
                format!("already at {short_latest}"),
                None,
            ));
            continue;
        }

        if !force {
            match has_uncommitted_changes(&instance.subrepo_path) {
                Ok(true) => {
                    println!("  ⚠️  {} (uncommitted changes)", instance.parent_repo);
                    outcomes.push(NestedOutcome::new(
                        instance,
                        NestedOutcomeKind::Skipped,
                        "uncommitted changes",
                        Some("commit or stash the local changes, then retry".to_string()),
                    ));
                    continue;
                }
                Ok(false) => {}
                Err(error) => {
                    let error = clean_error_message(&error.to_string());
                    println!("  ❌ {} (status failed: {})", instance.parent_repo, error);
                    outcomes.push(NestedOutcome::new(
                        instance,
                        NestedOutcomeKind::Failed,
                        format!("status failed: {error}"),
                        Some("inspect this nested repository, then retry".to_string()),
                    ));
                    continue;
                }
            }
        }

        if let Err(error) = ensure_commit_available(&instance.subrepo_path, &latest) {
            let error = clean_error_message(&error.to_string());
            println!("  ❌ {} (fetch failed: {})", instance.parent_repo, error);
            outcomes.push(NestedOutcome::new(
                instance,
                NestedOutcomeKind::Failed,
                format!("fetch failed: {error}"),
                Some("check the nested remote and authentication, then retry".to_string()),
            ));
            continue;
        }

        if !force {
            match is_ancestor(&instance.subrepo_path, &current_commit, &latest) {
                Ok(true) => {}
                Ok(false) => {
                    println!(
                        "  ⚠️  {} (local commits diverge from remote)",
                        instance.parent_repo
                    );
                    outcomes.push(NestedOutcome::new(
                        instance,
                        NestedOutcomeKind::Skipped,
                        "local commits diverge from remote",
                        Some(
                            "review the local commits and choose a target with `repos nested sync`"
                                .to_string(),
                        ),
                    ));
                    continue;
                }
                Err(error) => {
                    let error = clean_error_message(&error.to_string());
                    println!(
                        "  ❌ {} (history check failed: {})",
                        instance.parent_repo, error
                    );
                    outcomes.push(NestedOutcome::new(
                        instance,
                        NestedOutcomeKind::Failed,
                        format!("history check failed: {error}"),
                        Some("inspect the nested repository history, then retry".to_string()),
                    ));
                    continue;
                }
            }
        }
        candidates.push((instance, current_commit));
    }

    if outcomes
        .iter()
        .any(|outcome| outcome.kind == NestedOutcomeKind::Failed)
    {
        record_aborted_candidates(
            candidates.iter().map(|(instance, _)| *instance),
            &mut outcomes,
        );
        return finish_operation(NestedOperation::Update, &outcomes);
    }

    for (instance, old_commit) in candidates {
        match checkout_commit(&instance.subrepo_path, &latest, force) {
            Ok(()) => {
                let old_short = old_commit.chars().take(7).collect::<String>();
                println!(
                    "  ✅ {} ({} → {})",
                    instance.parent_repo, old_short, short_latest
                );
                outcomes.push(NestedOutcome::new(
                    instance,
                    NestedOutcomeKind::Changed,
                    format!("{old_short} → {short_latest}"),
                    None,
                ));
            }
            Err(error) => {
                let error = clean_error_message(&error.to_string());
                println!("  ❌ {} ({})", instance.parent_repo, error);
                outcomes.push(NestedOutcome::new(
                    instance,
                    NestedOutcomeKind::Failed,
                    error,
                    Some("resolve the checkout failure, then retry".to_string()),
                ));
            }
        }
    }

    finish_operation(NestedOperation::Update, &outcomes)
}

#[cfg(test)]
#[path = "sync/tests.rs"]
mod tests;
