//! Subrepo synchronization operations

use super::{get_current_commit, SubrepoInstance, ValidationReport};
use crate::core::{clean_error_message, format_relative_repo_path, truncate_text};
use crate::utils::compare_repository_locations;
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
        compare_repository_locations(&left.path, &left.repository, &right.path, &right.repository)
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

fn finish_operation(operation: NestedOperation, outcomes: &[NestedOutcome]) -> Result<()> {
    println!("\n{}\n", generate_operation_report(operation, outcomes));

    let error_count = outcomes
        .iter()
        .filter(|outcome| outcome.kind == NestedOutcomeKind::Failed)
        .count();
    if error_count > 0 {
        anyhow::bail!(
            "{error_count} repositories failed to {}",
            operation.command()
        );
    }
    Ok(())
}

fn record_aborted_candidates<'a>(
    candidates: impl IntoIterator<Item = &'a SubrepoInstance>,
    outcomes: &mut Vec<NestedOutcome>,
) {
    for instance in candidates {
        outcomes.push(NestedOutcome::new(
            instance,
            NestedOutcomeKind::Skipped,
            "batch preflight failed; no changes applied",
            Some("resolve the reported failure, then retry".to_string()),
        ));
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
fn checkout_commit(path: &Path, commit: &str, force: bool) -> Result<()> {
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

fn fetch_origin(path: &Path) -> Result<()> {
    let path_str = path_to_str(path)?;
    let fetch_output = Command::new("git")
        .args(["-C", path_str, "fetch", "origin"])
        .output()
        .context("Failed to run git fetch")?;

    if !fetch_output.status.success() {
        let stderr = String::from_utf8_lossy(&fetch_output.stderr);
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

fn ensure_commit_available(path: &Path, commit: &str) -> Result<()> {
    if commit_exists(path, commit)? {
        return Ok(());
    }
    fetch_origin(path)?;
    if commit_exists(path, commit)? {
        Ok(())
    } else {
        anyhow::bail!(
            "target commit {} is unavailable",
            commit.chars().take(7).collect::<String>()
        )
    }
}

/// Fetch from remote and determine one immutable update target.
fn fetch_latest_commit(path: &Path) -> Result<String> {
    let path_str = path_to_str(path)?;
    fetch_origin(path)?;

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
            checkout_kind: crate::subrepo::NestedCheckoutKind::Independent,
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
