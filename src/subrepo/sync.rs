//! Subrepo synchronization operations

use super::{SubrepoInstance, ValidationReport};
use crate::core::{clean_error_message, format_relative_repo_path, truncate_text};
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

const RESET: &str = "\x1b[0m";
const BOLD_BLUE: &str = "\x1b[1;38;5;75m";
const BOLD_PURPLE: &str = "\x1b[1;38;5;141m";
const GREEN: &str = "\x1b[1;38;5;114m";
const YELLOW: &str = "\x1b[1;38;5;221m";
const RED: &str = "\x1b[1;38;5;203m";
const DIM: &str = "\x1b[2m";

#[derive(Clone, Copy, Eq, PartialEq)]
enum NestedOperation {
    Sync,
    Update,
}

impl NestedOperation {
    fn command(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Update => "update",
        }
    }

    fn changed_label(self) -> &'static str {
        match self {
            Self::Sync => "Synced",
            Self::Update => "Updated",
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum NestedOutcomeKind {
    Changed,
    Unchanged,
    Skipped,
    Failed,
}

struct NestedOutcome {
    repository: String,
    path: String,
    kind: NestedOutcomeKind,
    message: String,
    next: Option<String>,
}

impl NestedOutcome {
    fn new(
        instance: &SubrepoInstance,
        kind: NestedOutcomeKind,
        message: impl Into<String>,
        next: Option<String>,
    ) -> Self {
        Self {
            repository: instance.parent_repo.clone(),
            path: instance.subrepo_path.to_string_lossy().into_owned(),
            kind,
            message: message.into(),
            next,
        }
    }
}

fn generate_operation_report(operation: NestedOperation, outcomes: &[NestedOutcome]) -> String {
    let mut outcomes = outcomes.iter().collect::<Vec<_>>();
    outcomes.sort_by(|left, right| {
        left.repository
            .cmp(&right.repository)
            .then_with(|| left.path.cmp(&right.path))
    });
    let count = |kind| {
        outcomes
            .iter()
            .filter(|outcome| outcome.kind == kind)
            .count()
    };
    let changed = count(NestedOutcomeKind::Changed);
    let unchanged = count(NestedOutcomeKind::Unchanged);
    let skipped = count(NestedOutcomeKind::Skipped);
    let failed = count(NestedOutcomeKind::Failed);

    let mut lines = vec![
        format!("{BOLD_BLUE}repos nested {}{RESET}", operation.command()),
        String::new(),
        format!("{BOLD_PURPLE}▌ Summary{RESET}"),
        format!(
            "  {GREEN}✓{RESET} {:<16}{changed}",
            operation.changed_label()
        ),
    ];
    if unchanged > 0 {
        lines.push(format!("  {GREEN}✓{RESET} {:<16}{unchanged}", "Up to date"));
    }
    if skipped > 0 {
        lines.push(format!("  {YELLOW}!{RESET} {:<16}{skipped}", "Skipped"));
    }
    if failed > 0 {
        lines.push(format!("  {RED}!{RESET} {:<16}{failed}", "Failed"));
    }
    lines.push(format!(
        "  {DIM}·{RESET} {:<16}{}",
        "Checked",
        outcomes.len()
    ));

    append_operation_section(
        &mut lines,
        &outcomes,
        NestedOutcomeKind::Changed,
        operation.changed_label(),
        GREEN,
        "✓",
    );
    append_operation_section(
        &mut lines,
        &outcomes,
        NestedOutcomeKind::Unchanged,
        "Up to Date",
        GREEN,
        "✓",
    );
    append_operation_section(
        &mut lines,
        &outcomes,
        NestedOutcomeKind::Skipped,
        "Skipped",
        YELLOW,
        "!",
    );
    append_operation_section(
        &mut lines,
        &outcomes,
        NestedOutcomeKind::Failed,
        "Failed",
        RED,
        "!",
    );

    lines.join("\n")
}

fn append_operation_section(
    lines: &mut Vec<String>,
    outcomes: &[&NestedOutcome],
    kind: NestedOutcomeKind,
    heading: &str,
    color: &str,
    marker: &str,
) {
    let matching = outcomes
        .iter()
        .filter(|outcome| outcome.kind == kind)
        .copied()
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return;
    }

    lines.push(String::new());
    lines.push(format!("{BOLD_PURPLE}▌ {heading}{RESET}"));
    for outcome in matching {
        lines.push(format!(
            "  {color}{marker}{RESET} {:24} {}",
            truncate_text(&outcome.repository, 24),
            outcome.message
        ));
        lines.push(format!(
            "    {DIM}↳ path: {}{RESET}",
            format_relative_repo_path(&outcome.path)
        ));
        if let Some(next) = &outcome.next {
            lines.push(format!("    {DIM}↳ next: {next}{RESET}"));
        }
    }
}

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

    let mut outcomes = Vec::with_capacity(instances.len());

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
        let mut stashed = false;

        // Handle uncommitted changes
        if has_changes {
            if stash {
                // Stash changes before syncing
                match stash_changes(&instance.subrepo_path) {
                    Ok(()) => {
                        stashed = true;
                    }
                    Err(e) => {
                        let error = clean_error_message(&e.to_string());
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
            } else if !force {
                // No stash, no force - skip
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
            // If force=true, proceed without stashing (will discard changes)
        }

        // Checkout the commit
        match checkout_commit(&instance.subrepo_path, target_commit) {
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

    println!(
        "\n{}\n",
        generate_operation_report(NestedOperation::Sync, &outcomes)
    );

    let error_count = outcomes
        .iter()
        .filter(|outcome| outcome.kind == NestedOutcomeKind::Failed)
        .count();
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

    let mut outcomes = Vec::with_capacity(instances.len());

    for instance in &instances {
        // Check if already at latest
        if instance.commit_hash == latest {
            println!("  ✨ {} (already at latest)", instance.parent_repo);
            outcomes.push(NestedOutcome::new(
                instance,
                NestedOutcomeKind::Unchanged,
                format!("already at {short_latest}"),
                None,
            ));
            continue;
        }

        // Check for uncommitted changes
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
                            outcomes.push(NestedOutcome::new(
                                instance,
                                NestedOutcomeKind::Skipped,
                                "local commits diverge from remote",
                                Some("review the local commits and choose a target with `repos nested sync`".to_string()),
                            ));
                            continue;
                        }
                        Err(e) => {
                            let error = clean_error_message(&e.to_string());
                            println!(
                                "  ❌ {} (history check failed: {})",
                                instance.parent_repo, error
                            );
                            outcomes.push(NestedOutcome::new(
                                instance,
                                NestedOutcomeKind::Failed,
                                format!("history check failed: {error}"),
                                Some(
                                    "inspect the nested repository history, then retry".to_string(),
                                ),
                            ));
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
                        outcomes.push(NestedOutcome::new(
                            instance,
                            NestedOutcomeKind::Changed,
                            format!("{old_short} → {short_latest}"),
                            None,
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
            Err(e) => {
                let error = clean_error_message(&e.to_string());
                println!("  ❌ {} (fetch failed: {})", instance.parent_repo, error);
                outcomes.push(NestedOutcome::new(
                    instance,
                    NestedOutcomeKind::Failed,
                    format!("fetch failed: {error}"),
                    Some("check the nested remote and authentication, then retry".to_string()),
                ));
            }
        }
    }

    println!(
        "\n{}\n",
        generate_operation_report(NestedOperation::Update, &outcomes)
    );

    let error_count = outcomes
        .iter()
        .filter(|outcome| outcome.kind == NestedOutcomeKind::Failed)
        .count();
    if error_count > 0 {
        anyhow::bail!("{error_count} repositories failed to update");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        find_instances_by_name, generate_operation_report, NestedOperation, NestedOutcome,
        NestedOutcomeKind,
    };
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

    #[test]
    fn final_nested_mutation_report_names_every_outcome() {
        let outcomes = vec![
            NestedOutcome {
                repository: "alpha".to_string(),
                path: "alpha/packages/shared".to_string(),
                kind: NestedOutcomeKind::Changed,
                message: "abc1234 → def5678".to_string(),
                next: None,
            },
            NestedOutcome {
                repository: "beta".to_string(),
                path: "beta/packages/shared".to_string(),
                kind: NestedOutcomeKind::Unchanged,
                message: "already at def5678".to_string(),
                next: None,
            },
            NestedOutcome {
                repository: "gamma".to_string(),
                path: "gamma/packages/shared".to_string(),
                kind: NestedOutcomeKind::Skipped,
                message: "uncommitted changes".to_string(),
                next: Some("commit or stash the local changes, then retry".to_string()),
            },
            NestedOutcome {
                repository: "omega".to_string(),
                path: "omega/packages/shared".to_string(),
                kind: NestedOutcomeKind::Failed,
                message: "fetch failed: authentication failed".to_string(),
                next: Some("check the nested remote and authentication, then retry".to_string()),
            },
        ];

        let report = generate_operation_report(NestedOperation::Update, &outcomes);

        assert!(report.contains("repos nested update"));
        assert!(report.contains("Updated         1"));
        assert!(report.contains("Up to date      1"));
        assert!(report.contains("Skipped         1"));
        assert!(report.contains("Failed          1"));
        assert!(report.contains("Checked         4"));
        for repository in ["alpha", "beta", "gamma", "omega"] {
            assert!(report.contains(repository));
            assert!(report.contains(&format!("path: ./{repository}/packages/shared")));
        }
        assert!(report.contains("next: commit or stash"));
        assert!(report.contains("next: check the nested remote and authentication"));
    }
}
