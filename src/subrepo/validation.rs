//! Validation logic for discovering nested repositories

use super::{
    get_commit_timestamp, get_current_commit, get_remote_url, has_uncommitted_changes,
    NestedCheckoutKind, SubrepoInstance, ValidationReport,
};
use crate::core::topology::{gitlink_target, normalize_path};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const RESET: &str = "\x1b[0m";
const BOLD_BLUE: &str = "\x1b[1;38;5;75m";
const BOLD_PURPLE: &str = "\x1b[1;38;5;141m";
const GREEN: &str = "\x1b[1;38;5;114m";
const YELLOW: &str = "\x1b[1;38;5;221m";
const DIM: &str = "\x1b[2m";

/// Discover all nested repositories and generate a validation report
pub fn validate_subrepos() -> Result<ValidationReport> {
    Ok(validate_subrepos_inventory(true)?.0)
}

/// Discover all nested repositories without printing scan progress.
pub fn validate_subrepos_quiet() -> Result<ValidationReport> {
    Ok(validate_subrepos_inventory(false)?.0)
}

pub(crate) fn validate_subrepos_inventory(show_scan: bool) -> Result<(ValidationReport, usize)> {
    let repositories = crate::core::discovery::find_repos();
    let fleet_repositories = repositories.len();
    Ok((
        validate_discovered_repositories(&repositories, show_scan)?,
        fleet_repositories,
    ))
}

pub(crate) fn validate_discovered_repositories(
    repositories: &[(String, PathBuf)],
    show_scan: bool,
) -> Result<ValidationReport> {
    let mut all_nested = Vec::new();

    if show_scan {
        println!(
            "🔍 Inspecting {} fleet repositories for nested checkouts...\n",
            repositories.len()
        );
    }

    let normalized = repositories
        .iter()
        .map(|(_, path)| normalize_path(path))
        .collect::<Vec<_>>();
    for (child_index, (_, child_path)) in repositories.iter().enumerate() {
        let Some(parent_index) = nearest_parent_index(child_index, &normalized) else {
            continue;
        };

        let (parent_name, parent_path) = &repositories[parent_index];
        let relative_path = relative_repository_path(
            child_path,
            parent_path,
            &normalized[child_index],
            &normalized[parent_index],
        );
        let checkout_kind = if gitlink_target(
            parent_path,
            &normalized[parent_index],
            &normalized[child_index],
        )
        .is_some()
        {
            NestedCheckoutKind::Submodule
        } else if child_path.join(".git").is_file() {
            NestedCheckoutKind::LinkedWorktree
        } else {
            NestedCheckoutKind::Independent
        };

        all_nested.push(inspect_nested_repository(
            parent_name,
            parent_path,
            child_path,
            relative_path,
            checkout_kind,
        )?);
    }

    // Group by remote URL
    let mut by_remote: HashMap<String, Vec<SubrepoInstance>> = HashMap::new();
    let mut no_remote = Vec::new();

    for instance in all_nested {
        if let Some(ref remote) = instance.remote_url {
            by_remote.entry(remote.clone()).or_default().push(instance);
        } else {
            no_remote.push(instance);
        }
    }

    let total_nested = by_remote.values().map(std::vec::Vec::len).sum::<usize>() + no_remote.len();

    Ok(ValidationReport {
        total_nested,
        by_remote,
        no_remote,
    })
}

fn nearest_parent_index(child: usize, normalized: &[PathBuf]) -> Option<usize> {
    (0..normalized.len())
        .filter(|parent| {
            *parent != child
                && normalized[child] != normalized[*parent]
                && normalized[child].starts_with(&normalized[*parent])
        })
        .max_by_key(|parent| normalized[*parent].components().count())
}

fn relative_repository_path(
    child_path: &Path,
    parent_path: &Path,
    normalized_child: &Path,
    normalized_parent: &Path,
) -> String {
    child_path
        .strip_prefix(parent_path)
        .or_else(|_| normalized_child.strip_prefix(normalized_parent))
        .unwrap_or(child_path)
        .to_string_lossy()
        .to_string()
}

fn inspect_nested_repository(
    parent_name: &str,
    parent_path: &Path,
    subrepo_path: &Path,
    relative_path: String,
    checkout_kind: NestedCheckoutKind,
) -> Result<SubrepoInstance> {
    let subrepo_name = subrepo_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");
    let commit_hash = get_current_commit(subrepo_path).with_context(|| {
        format!(
            "failed to inspect HEAD for nested repository {}",
            subrepo_path.display()
        )
    })?;
    let short_hash = commit_hash.chars().take(7).collect();
    let remote_url = get_remote_url(subrepo_path).with_context(|| {
        format!(
            "failed to inspect origin for nested repository {}",
            subrepo_path.display()
        )
    })?;
    let has_uncommitted = has_uncommitted_changes(subrepo_path).with_context(|| {
        format!(
            "failed to inspect worktree for nested repository {}",
            subrepo_path.display()
        )
    })?;
    let commit_timestamp = get_commit_timestamp(subrepo_path, &commit_hash).with_context(|| {
        format!(
            "failed to inspect commit timestamp for nested repository {}",
            subrepo_path.display()
        )
    })?;

    Ok(SubrepoInstance {
        parent_repo: parent_name.to_string(),
        parent_path: parent_path.to_path_buf(),
        subrepo_name: subrepo_name.to_string(),
        subrepo_path: subrepo_path.to_path_buf(),
        relative_path,
        commit_hash,
        short_hash,
        remote_url,
        has_uncommitted,
        commit_timestamp,
        checkout_kind,
    })
}

/// Display the validation report
pub fn display_report(report: &ValidationReport) {
    println!("{}", generate_validation_report(report));
}

fn generate_validation_report(report: &ValidationReport) -> String {
    let shared = report.shared_subrepos_count();
    let unique = report.unique_remotes().saturating_sub(shared);
    let mut lines = vec![
        format!("{BOLD_BLUE}repos nested validate{RESET}"),
        String::new(),
        format!("{BOLD_PURPLE}▌ Summary{RESET}"),
        format!("  {GREEN}✓{RESET} {:<18}{shared}", "Shared groups"),
        format!("  {DIM}·{RESET} {:<18}{unique}", "Unique groups"),
    ];
    if !report.no_remote.is_empty() {
        lines.push(format!(
            "  {YELLOW}!{RESET} {:<18}{}",
            "Missing remote",
            report.no_remote.len()
        ));
    }
    lines.push(format!(
        "  {DIM}·{RESET} {:<18}{}",
        "Nested copies", report.total_nested
    ));
    lines.push(format!(
        "  {DIM}·{RESET} {:<18}{}",
        "Independent",
        report.checkout_count(NestedCheckoutKind::Independent)
    ));
    lines.push(format!(
        "  {DIM}·{RESET} {:<18}{}",
        "Submodules",
        report.checkout_count(NestedCheckoutKind::Submodule)
    ));
    lines.push(format!(
        "  {DIM}·{RESET} {:<18}{}",
        "Linked worktrees",
        report.checkout_count(NestedCheckoutKind::LinkedWorktree)
    ));

    if report.total_nested == 0 {
        lines.push(String::new());
        lines.push(format!("{BOLD_PURPLE}▌ Result{RESET}"));
        lines.push(format!("  {DIM}No nested repositories found.{RESET}"));
        return lines.join("\n");
    }

    let mut groups = report.by_remote.iter().collect::<Vec<_>>();
    groups.sort_by_key(|(remote, _)| *remote);
    if !groups.is_empty() {
        lines.push(String::new());
        lines.push(format!("{BOLD_PURPLE}▌ Nested Repositories{RESET}"));
        for (remote, instances) in groups {
            let mut instances = instances.iter().collect::<Vec<_>>();
            instances.sort_by(|left, right| {
                left.parent_repo
                    .cmp(&right.parent_repo)
                    .then_with(|| left.relative_path.cmp(&right.relative_path))
            });
            let name = &instances[0].subrepo_name;
            let copies = instances.len();
            let group_kind = if copies > 1 { "shared" } else { "unique" };
            lines.push(format!(
                "  {GREEN}✓{RESET} {name} ({copies} copies, {group_kind})"
            ));
            lines.push(format!("    {DIM}↳ remote: {remote}{RESET}"));
            for instance in instances {
                let state = if instance.has_uncommitted {
                    "uncommitted changes"
                } else {
                    "clean"
                };
                lines.push(format!(
                    "    · {} @ {} ({state}, {})",
                    nested_location(instance),
                    instance.short_hash,
                    instance.checkout_kind.label()
                ));
            }
        }
    }

    if !report.no_remote.is_empty() {
        let mut missing = report.no_remote.iter().collect::<Vec<_>>();
        missing.sort_by(|left, right| {
            left.parent_repo
                .cmp(&right.parent_repo)
                .then_with(|| left.relative_path.cmp(&right.relative_path))
        });
        lines.push(String::new());
        lines.push(format!("{BOLD_PURPLE}▌ Missing Remote{RESET}"));
        for instance in missing {
            lines.push(format!(
                "  {YELLOW}!{RESET} {} @ {} ({})",
                nested_location(instance),
                instance.short_hash,
                instance.checkout_kind.label()
            ));
            lines.push(format!(
                "    {DIM}↳ next: add an origin remote or exclude this nested repository{RESET}"
            ));
        }
    }

    lines.push(String::new());
    lines.push(format!("{BOLD_PURPLE}▌ Result{RESET}"));
    if shared > 0 {
        lines.push(format!(
            "  {GREEN}✓{RESET} Drift tracking applies to {shared} shared nested group{}.",
            if shared == 1 { "" } else { "s" }
        ));
        lines.push(format!(
            "    {DIM}↳ next: run `repos nested status` to inspect commit drift{RESET}"
        ));
    } else {
        lines.push(format!(
            "  {DIM}No nested remote is shared across parent repositories, so cross-copy drift cannot occur.{RESET}"
        ));
    }

    lines.join("\n")
}

fn nested_location(instance: &SubrepoInstance) -> String {
    match instance.relative_path.as_str() {
        "" | "." => instance.parent_repo.clone(),
        relative => format!("{}/{relative}", instance.parent_repo),
    }
}

#[cfg(test)]
mod tests {
    use super::{generate_validation_report, validate_discovered_repositories};
    use crate::subrepo::{NestedCheckoutKind, SubrepoInstance, ValidationReport};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    fn instance(parent: &str, relative: &str, dirty: bool) -> SubrepoInstance {
        SubrepoInstance {
            parent_repo: parent.to_string(),
            parent_path: PathBuf::from(parent),
            subrepo_name: "shared".to_string(),
            subrepo_path: PathBuf::from(parent).join(relative),
            relative_path: relative.to_string(),
            commit_hash: format!("{parent}-commit"),
            short_hash: format!("{parent}123"),
            remote_url: Some("github.com/team/shared".to_string()),
            has_uncommitted: dirty,
            commit_timestamp: 0,
            checkout_kind: NestedCheckoutKind::Independent,
        }
    }

    fn initialize_repository(path: &std::path::Path) {
        std::fs::create_dir_all(path).unwrap();
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(path)
            .status()
            .unwrap()
            .success());
        assert!(Command::new("git")
            .args([
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--allow-empty",
                "-q",
                "-m",
                "initial",
            ])
            .current_dir(path)
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn validation_report_is_sorted_attributable_and_actionable() {
        let report = ValidationReport {
            total_nested: 3,
            by_remote: HashMap::from([(
                "github.com/team/shared".to_string(),
                vec![
                    instance("zeta", "packages/shared", false),
                    instance("alpha", "vendor/shared", true),
                ],
            )]),
            no_remote: vec![instance("orphan", "nested/shared", false)],
        };

        let output = generate_validation_report(&report);

        assert!(output.contains("repos nested validate"));
        assert!(output.contains("Shared groups     1"));
        assert!(output.contains("Missing remote    1"));
        assert!(output.contains("Nested copies     3"));
        assert!(output.contains("alpha/vendor/shared"));
        assert!(output.contains("zeta/packages/shared"));
        assert!(output.contains("orphan/nested/shared"));
        assert!(output.contains("next: add an origin remote"));
        assert!(output.contains("next: run `repos nested status`"));
        assert!(!output.contains("BUILD IT"));
        assert!(!output.contains("SKIP IT"));
    }

    #[test]
    fn inventory_assigns_each_checkout_to_its_nearest_parent_once() {
        let temp_dir = TempDir::new().unwrap();
        let parent = temp_dir.path().join("parent");
        let child = parent.join("child");
        let grandchild = child.join("grandchild");
        initialize_repository(&parent);
        initialize_repository(&child);
        initialize_repository(&grandchild);

        let report = validate_discovered_repositories(
            &[
                ("parent".to_string(), parent.clone()),
                ("child".to_string(), child.clone()),
                ("grandchild".to_string(), grandchild),
            ],
            false,
        )
        .unwrap();

        assert_eq!(report.total_nested, 2);
        assert_eq!(report.no_remote.len(), 2);
        let child_instance = report
            .no_remote
            .iter()
            .find(|instance| instance.subrepo_name == "child")
            .unwrap();
        assert_eq!(child_instance.parent_repo, "parent");
        assert_eq!(child_instance.relative_path, "child");
        let grandchild_instance = report
            .no_remote
            .iter()
            .find(|instance| instance.subrepo_name == "grandchild")
            .unwrap();
        assert_eq!(grandchild_instance.parent_repo, "child");
        assert_eq!(grandchild_instance.relative_path, "grandchild");
    }

    #[test]
    fn inventory_classifies_registered_git_submodules_even_with_embedded_git_directory() {
        let temp_dir = TempDir::new().unwrap();
        let parent = temp_dir.path().join("parent");
        let submodule = parent.join("submodule");
        initialize_repository(&parent);
        initialize_repository(&submodule);
        assert!(Command::new("git")
            .args(["add", "submodule"])
            .current_dir(&parent)
            .output()
            .unwrap()
            .status
            .success());

        let report = validate_discovered_repositories(
            &[
                ("parent".to_string(), parent),
                ("submodule".to_string(), submodule),
            ],
            false,
        )
        .unwrap();

        assert_eq!(report.total_nested, 1);
        assert_eq!(report.no_remote.len(), 1);
        assert_eq!(
            report.no_remote[0].checkout_kind,
            NestedCheckoutKind::Submodule
        );
    }

    #[test]
    fn inventory_includes_standard_gitfile_submodules() {
        let temp_dir = TempDir::new().unwrap();
        let source = temp_dir.path().join("source");
        let parent = temp_dir.path().join("parent");
        initialize_repository(&source);
        initialize_repository(&parent);
        assert!(Command::new("git")
            .args([
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                source.to_str().unwrap(),
                "modules/shared",
            ])
            .current_dir(&parent)
            .output()
            .unwrap()
            .status
            .success());
        let submodule = parent.join("modules/shared");
        assert!(submodule.join(".git").is_file());

        let report = validate_discovered_repositories(
            &[
                ("parent".to_string(), parent),
                ("shared".to_string(), submodule),
            ],
            false,
        )
        .unwrap();

        assert_eq!(report.total_nested, 1);
        assert_eq!(report.checkout_count(NestedCheckoutKind::Submodule), 1);
    }

    #[test]
    fn inventory_includes_linked_worktrees_without_calling_them_submodules() {
        let temp_dir = TempDir::new().unwrap();
        let parent = temp_dir.path().join("parent");
        initialize_repository(&parent);
        let worktree = parent.join("worktrees/preview");
        assert!(Command::new("git")
            .args(["worktree", "add", "--detach"])
            .arg(&worktree)
            .current_dir(&parent)
            .output()
            .unwrap()
            .status
            .success());
        assert!(worktree.join(".git").is_file());

        let report = validate_discovered_repositories(
            &[
                ("parent".to_string(), parent),
                ("preview".to_string(), worktree),
            ],
            false,
        )
        .unwrap();

        assert_eq!(report.total_nested, 1);
        assert_eq!(report.checkout_count(NestedCheckoutKind::LinkedWorktree), 1);
        assert_eq!(report.checkout_count(NestedCheckoutKind::Submodule), 0);
    }
}
