//! Validation logic for discovering nested repositories

use super::{
    get_commit_timestamp, get_current_commit, get_remote_url, has_uncommitted_changes,
    SubrepoInstance, ValidationReport,
};
use crate::core::config::{FLEET_IGNORE_FILENAME, SKIP_DIRECTORIES};
use anyhow::Result;
use ignore::WalkBuilder;
use std::collections::HashMap;
use std::path::Path;

const RESET: &str = "\x1b[0m";
const BOLD_BLUE: &str = "\x1b[1;38;5;75m";
const BOLD_PURPLE: &str = "\x1b[1;38;5;141m";
const GREEN: &str = "\x1b[1;38;5;114m";
const YELLOW: &str = "\x1b[1;38;5;221m";
const DIM: &str = "\x1b[2m";

/// Discover all nested repositories and generate a validation report
pub fn validate_subrepos() -> Result<ValidationReport> {
    validate_subrepos_with_output(true)
}

/// Discover all nested repositories without printing scan progress.
pub fn validate_subrepos_quiet() -> Result<ValidationReport> {
    validate_subrepos_with_output(false)
}

fn validate_subrepos_with_output(show_scan: bool) -> Result<ValidationReport> {
    let parent_repos = crate::core::discovery::find_repos();
    let mut all_nested = Vec::new();

    if show_scan {
        println!(
            "🔍 Scanning {} parent repositories for nested repos...\n",
            parent_repos.len()
        );
    }

    for (parent_name, parent_path) in parent_repos {
        let nested = find_nested_in_parent(&parent_name, &parent_path)?;
        all_nested.extend(nested);
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

/// Find nested repositories within a parent repository
fn find_nested_in_parent(parent_name: &str, parent_path: &Path) -> Result<Vec<SubrepoInstance>> {
    let mut nested = Vec::new();

    // Walk the parent looking for nested .git directories
    let walker = WalkBuilder::new(parent_path)
        .parents(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .add_custom_ignore_filename(FLEET_IGNORE_FILENAME)
        .follow_links(false)
        .max_depth(Some(5)) // Don't go too deep
        .filter_entry(|entry| {
            let file_name = entry.file_name().to_str().unwrap_or("");

            // Skip build/dependency directories
            if SKIP_DIRECTORIES.contains(&file_name) {
                return false;
            }

            // Skip .git directories themselves from walking
            if file_name == ".git" {
                return false;
            }

            true
        })
        .build();

    for entry in walker.flatten() {
        let path = entry.path();

        // Only check directories
        if !entry.file_type().is_some_and(|ft| ft.is_dir()) {
            continue;
        }

        // Skip the parent's root directory
        if path == parent_path {
            continue;
        }

        // Nested drift management is for independent embedded repositories.
        // Submodules and linked worktrees use a .git file and are handled by
        // fleet topology/Git itself instead.
        let git_path = path.join(".git");
        if !git_path.is_dir() {
            continue;
        }

        // This is a nested repo! Get its info
        let subrepo_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let relative_path = path
            .strip_prefix(parent_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        // Get git info
        let commit_hash = match get_current_commit(path) {
            Ok(hash) => hash,
            Err(_) => continue, // Skip if can't get commit
        };

        let short_hash = commit_hash.chars().take(7).collect();
        let remote_url = get_remote_url(path).ok();
        let uncommitted = has_uncommitted_changes(path)?;
        let commit_timestamp = get_commit_timestamp(path, &commit_hash);

        nested.push(SubrepoInstance {
            parent_repo: parent_name.to_string(),
            parent_path: parent_path.to_path_buf(),
            subrepo_name,
            subrepo_path: path.to_path_buf(),
            relative_path,
            commit_hash,
            short_hash,
            remote_url,
            has_uncommitted: uncommitted,
            commit_timestamp,
        });
    }

    Ok(nested)
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
        "Checked", report.total_nested
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
                    "    · {} @ {} ({state})",
                    nested_location(instance),
                    instance.short_hash
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
                "  {YELLOW}!{RESET} {} @ {}",
                nested_location(instance),
                instance.short_hash
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
    use super::generate_validation_report;
    use crate::subrepo::{SubrepoInstance, ValidationReport};
    use std::collections::HashMap;
    use std::path::PathBuf;

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
        }
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
        assert!(output.contains("Checked           3"));
        assert!(output.contains("alpha/vendor/shared"));
        assert!(output.contains("zeta/packages/shared"));
        assert!(output.contains("orphan/nested/shared"));
        assert!(output.contains("next: add an origin remote"));
        assert!(output.contains("next: run `repos nested status`"));
        assert!(!output.contains("BUILD IT"));
        assert!(!output.contains("SKIP IT"));
    }
}
