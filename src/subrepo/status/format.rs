//! Concise nested-drift formatting for fleet reports.

use super::style::{paint, BOLD_PURPLE, DIM, GREEN, RED, RESET, YELLOW};
use super::{
    compare_package_statuses, instance_location, select_sync_target, NestedStatusReport,
    SubrepoStatus,
};
use crate::core::{clean_error_message, truncate_text};
use crate::subrepo::{NestedCheckoutKind, SubrepoInstance};

/// Display concise drift summary for use in `repos push`.
pub fn display_drift_summary(statuses: &[SubrepoStatus]) {
    for line in format_drift_section(statuses) {
        println!("{line}");
    }
}

/// Format nested drift with its own report section header.
#[must_use]
pub fn format_drift_section(statuses: &[SubrepoStatus]) -> Vec<String> {
    format_drift_summary_lines(statuses, None)
}

/// Format nested drift as actionable work items for another report section.
#[must_use]
pub fn format_drift_work_items(statuses: &[SubrepoStatus]) -> (usize, Vec<String>) {
    let drifted_count = statuses.iter().filter(|status| status.has_drift).count();
    (drifted_count, format_drift_summary_lines(statuses, None))
}

/// Format drift with explicit inventory coverage for fleet command reports.
#[must_use]
pub fn format_drift_work_items_with_inventory(report: &NestedStatusReport) -> (usize, Vec<String>) {
    (
        report.drifted_count(),
        format_drift_summary_lines(&report.groups, Some(report)),
    )
}

/// Format a failed automatic drift inspection instead of implying no drift.
#[must_use]
pub fn format_drift_failure(error: &anyhow::Error) -> Vec<String> {
    vec![
        format!("{BOLD_PURPLE}▌ Nested Package Drift{RESET}"),
        format!(
            "{RED}!{RESET} Drift check incomplete: {}",
            clean_error_message(&error.to_string())
        ),
        format!("{DIM}↳ Run `repos nested validate` to inspect the failure.{RESET}"),
    ]
}

fn format_drift_summary_lines(
    statuses: &[SubrepoStatus],
    inventory: Option<&NestedStatusReport>,
) -> Vec<String> {
    let mut drifted = statuses
        .iter()
        .filter(|status| status.has_drift)
        .collect::<Vec<_>>();
    drifted.sort_by(|left, right| compare_package_statuses(left, right));
    let has_drift = !drifted.is_empty();

    let mut lines = vec![format!("{BOLD_PURPLE}▌ Nested Package Drift{RESET}")];
    if let Some(inventory) = inventory {
        let shared_groups = inventory.shared_group_count();
        let group_label = if shared_groups == 1 {
            "group"
        } else {
            "groups"
        };
        if inventory.total_nested == 0 {
            lines.push(format!(
                "{GREEN}✓{RESET} No nested checkouts discovered in {} fleet repositories",
                inventory.fleet_repositories
            ));
        } else if shared_groups == 0 {
            lines.push(format!(
                "{GREEN}✓{RESET} No shared nested package groups to compare"
            ));
        } else if drifted.is_empty() {
            lines.push(format!(
                "{GREEN}✓{RESET} No commit drift across {shared_groups} shared nested package {group_label}"
            ));
        } else {
            let verb = if drifted.len() == 1 { "is" } else { "are" };
            lines.push(format!(
                "{YELLOW}!{RESET} {} of {shared_groups} shared nested package {group_label} {verb} at different commits",
                drifted.len()
            ));
        }
        lines.push(format!(
            "{DIM}· Compared {} shared copies across {} fleet repositories; {} unique and {} missing-origin copies are not drift-comparable{RESET}",
            inventory.shared_copy_count(),
            inventory.fleet_repositories,
            inventory.unique_groups().count(),
            inventory.no_remote.len()
        ));
        lines.push(format!(
            "{DIM}· Scope: {} independent, {} submodule, {} linked-worktree copies{RESET}",
            inventory.checkout_count(NestedCheckoutKind::Independent),
            inventory.checkout_count(NestedCheckoutKind::Submodule),
            inventory.checkout_count(NestedCheckoutKind::LinkedWorktree),
        ));
    } else {
        if drifted.is_empty() {
            return Vec::new();
        }
        let group_label = if drifted.len() == 1 {
            "group is"
        } else {
            "groups are"
        };
        lines.push(format!(
            "{YELLOW}!{RESET} {} nested package {group_label} at different commits",
            drifted.len()
        ));
    }

    for status in drifted {
        format_drift_summary_item(status, &mut lines);
    }
    if has_drift {
        lines.push(format!(
            "{DIM}↳ Run `repos nested status` for per-copy details.{RESET}"
        ));
    }
    lines
}

fn format_drift_summary_item(status: &SubrepoStatus, lines: &mut Vec<String>) {
    let Some(target) = select_sync_target(status) else {
        return;
    };

    let scoped_suffix = format!("/@goobits/{}", status.name);
    let package_label = if status
        .instances
        .iter()
        .any(|instance| instance.relative_path.contains(&scoped_suffix))
    {
        format!("@goobits/{}", status.name)
    } else {
        format!("pkg:{}", status.name)
    };
    lines.push(format!(
        "  {:22} {:>2} copies  → repos nested sync {} --to {}",
        truncate_text(&package_label, 22),
        status.instances.len(),
        status.name,
        target.short_hash
    ));

    let mut rows = status
        .instances
        .iter()
        .map(|instance| DriftRow {
            state: DriftState::from_instance(instance, &target.commit_hash),
            location: instance_location(instance),
            short_hash: instance.short_hash.clone(),
            checkout_kind: instance.checkout_kind,
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.location
            .cmp(&right.location)
            .then_with(|| left.short_hash.cmp(&right.short_hash))
    });

    for row in rows {
        let state_cell = format!("{:<8}", row.state.label());
        lines.push(format!(
            "    {} {:30} {}  {}",
            paint(row.state.color(), &state_cell),
            truncate_text(&row.location, 30),
            row.short_hash,
            row.checkout_kind.label(),
        ));
    }
}

struct DriftRow {
    state: DriftState,
    location: String,
    short_hash: String,
    checkout_kind: NestedCheckoutKind,
}

#[derive(Clone, Copy)]
enum DriftState {
    Target,
    Update,
    Dirty,
    DirtyTarget,
}

impl DriftState {
    fn from_instance(instance: &SubrepoInstance, target_hash: &str) -> Self {
        match (
            instance.has_uncommitted,
            instance.commit_hash == target_hash,
        ) {
            (true, true) => Self::DirtyTarget,
            (true, false) => Self::Dirty,
            (false, true) => Self::Target,
            (false, false) => Self::Update,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Target => "✓ target",
            Self::Update => "→ sync",
            Self::Dirty => "! dirty",
            Self::DirtyTarget => "! target",
        }
    }

    fn color(self) -> &'static str {
        match self {
            Self::Target => GREEN,
            Self::Update => YELLOW,
            Self::Dirty | Self::DirtyTarget => RED,
        }
    }
}
