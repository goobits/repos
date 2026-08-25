//! High-level terminal layout for nested-status reports.

use super::detail::{
    display_drift_status, display_missing_remote, display_synced_status, display_unique_status,
};
use super::style::{BOLD_BLUE, BOLD_PURPLE, DIM, GREEN, RESET, YELLOW};
use super::{instance_location, NestedStatusReport, SubrepoInstance, SubrepoStatus};
use crate::subrepo::NestedCheckoutKind;

/// Display shared subrepo status through the compatibility API.
pub fn display_status(statuses: &[SubrepoStatus], show_all: bool) {
    let total_nested = statuses.iter().map(|status| status.instances.len()).sum();
    display_status_inventory(statuses, &[], total_nested, None, show_all);
}

/// Display a complete independent-nested-repository inventory.
pub fn display_nested_status(report: &NestedStatusReport, show_all: bool) {
    display_status_inventory(
        &report.groups,
        &report.no_remote,
        report.total_nested,
        Some(report.fleet_repositories),
        show_all,
    );
}

fn display_status_inventory(
    statuses: &[SubrepoStatus],
    no_remote: &[SubrepoInstance],
    total_nested: usize,
    fleet_repositories: Option<usize>,
    show_all: bool,
) {
    println!(
        "\n{}",
        generate_status_summary(statuses, no_remote, total_nested, fleet_repositories)
    );

    if total_nested == 0 {
        println!("\n{BOLD_PURPLE}▌ Result{RESET}");
        if fleet_repositories.is_some() {
            println!("  {DIM}No nested repositories found.{RESET}\n");
        } else {
            println!("  {DIM}No shared nested repository statuses supplied.{RESET}");
            println!(
                "  {DIM}Complete fleet coverage is unavailable through this legacy view.{RESET}\n"
            );
        }
        return;
    }

    let drifted = statuses
        .iter()
        .filter(|status| status.instances.len() > 1 && status.has_drift)
        .collect::<Vec<_>>();
    let synced = statuses
        .iter()
        .filter(|status| status.instances.len() > 1 && !status.has_drift)
        .collect::<Vec<_>>();
    let unique = statuses
        .iter()
        .filter(|status| status.instances.len() == 1)
        .collect::<Vec<_>>();

    println!();
    display_section("🔴 NESTED DRIFT", &drifted, display_drift_status);
    if show_all {
        display_section("🟢 SYNCED SHARED GROUPS", &synced, display_synced_status);
        display_section(
            "⚪ UNIQUE NESTED REPOSITORIES",
            &unique,
            display_unique_status,
        );
        display_missing_remote_section(no_remote);
    } else if !synced.is_empty() || !unique.is_empty() || !no_remote.is_empty() {
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        println!(
            "💡 Details hidden: {} synced shared groups, {} unique repositories, {} missing origin",
            synced.len(),
            unique.len(),
            no_remote.len()
        );
        println!("   Use --all to show every discovered nested repository");
        println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    }

    if !drifted.is_empty() {
        println!();
        println!("🔧 To update a drifted repo to its 'origin/main' branch instead, run:");
        println!("   repos nested update <name>  (e.g., 'repos nested update docs-engine')");
    }
    println!();
}

fn display_section(title: &str, statuses: &[&SubrepoStatus], display_item: fn(&SubrepoStatus)) {
    if statuses.is_empty() {
        return;
    }

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("{title} ({})", statuses.len());
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    for status in statuses {
        display_item(status);
    }
}

fn display_missing_remote_section(instances: &[SubrepoInstance]) {
    if instances.is_empty() {
        return;
    }

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("🟡 MISSING ORIGIN ({})", instances.len());
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    let mut instances = instances.iter().collect::<Vec<_>>();
    instances.sort_by_key(|instance| instance_location(instance));
    for instance in instances {
        display_missing_remote(instance);
    }
}

pub(super) fn generate_status_summary(
    statuses: &[SubrepoStatus],
    no_remote: &[SubrepoInstance],
    total_nested: usize,
    fleet_repositories: Option<usize>,
) -> String {
    let missing_remote = no_remote.len();
    let shared = statuses
        .iter()
        .filter(|status| status.instances.len() > 1)
        .count();
    let unique = statuses
        .iter()
        .filter(|status| status.instances.len() == 1)
        .count();
    let drifted = statuses
        .iter()
        .filter(|status| status.instances.len() > 1 && status.has_drift)
        .count();
    let synced = shared.saturating_sub(drifted);
    let mut lines = vec![
        format!("{BOLD_BLUE}repos nested status{RESET}"),
        String::new(),
        format!("{BOLD_PURPLE}▌ Summary{RESET}"),
        format!("  {YELLOW}!{RESET} {:<18}{drifted}", "Drifted groups"),
        format!("  {GREEN}✓{RESET} {:<18}{synced}", "Synced groups"),
        format!("  {DIM}·{RESET} {:<18}{shared}", "Shared groups"),
    ];
    if fleet_repositories.is_some() {
        lines.push(format!("  {DIM}·{RESET} {:<18}{unique}", "Unique groups"));
    }
    if fleet_repositories.is_some() && missing_remote > 0 {
        lines.push(format!(
            "  {YELLOW}!{RESET} {:<18}{missing_remote}",
            "Missing origin"
        ));
    }
    if let Some(fleet_repositories) = fleet_repositories {
        let checkout_count = |kind| {
            statuses
                .iter()
                .flat_map(|status| status.instances.iter())
                .chain(no_remote.iter())
                .filter(|instance| instance.checkout_kind == kind)
                .count()
        };
        lines.push(format!(
            "  {DIM}·{RESET} {:<18}{total_nested}",
            "Nested copies"
        ));
        lines.push(format!(
            "  {DIM}·{RESET} {:<18}{}",
            "Independent",
            checkout_count(NestedCheckoutKind::Independent)
        ));
        lines.push(format!(
            "  {DIM}·{RESET} {:<18}{}",
            "Submodules",
            checkout_count(NestedCheckoutKind::Submodule)
        ));
        lines.push(format!(
            "  {DIM}·{RESET} {:<18}{}",
            "Linked worktrees",
            checkout_count(NestedCheckoutKind::LinkedWorktree)
        ));
        lines.push(format!(
            "  {DIM}·{RESET} {:<18}{fleet_repositories}",
            "Fleet repos"
        ));
        lines.push(format!(
            "  {DIM}Scope: every discovered nested checkout{RESET}"
        ));
    } else {
        lines.push(format!(
            "  {DIM}·{RESET} {:<18}{total_nested}",
            "Shared copies"
        ));
        lines.push(format!(
            "  {DIM}Scope: shared groups supplied by caller; complete fleet coverage unavailable{RESET}"
        ));
    }
    lines.join("\n")
}
