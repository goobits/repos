use super::display::generate_status_summary;
use super::{
    analyze_nested_status_for_repositories, format_drift_failure, format_drift_section,
    format_drift_work_items_with_inventory, NestedStatusReport, SubrepoStatus,
};
use crate::subrepo::{NestedCheckoutKind, SubrepoInstance};
use std::path::PathBuf;

fn instance(
    parent: &str,
    package: &str,
    commit: &str,
    short: &str,
    dirty: bool,
    timestamp: i64,
) -> SubrepoInstance {
    SubrepoInstance {
        parent_repo: parent.to_string(),
        parent_path: PathBuf::from(parent),
        subrepo_name: package.to_string(),
        subrepo_path: PathBuf::from(parent).join(format!("packages/{package}")),
        relative_path: format!("packages/{package}"),
        commit_hash: commit.to_string(),
        short_hash: short.to_string(),
        remote_url: Some("github.com/team/shared".to_string()),
        has_uncommitted: dirty,
        commit_timestamp: timestamp,
        checkout_kind: NestedCheckoutKind::Independent,
    }
}

#[test]
fn nested_status_contract_counts_groups_and_names_each_drifted_copy() {
    let drifted = SubrepoStatus::new(
        "shared".to_string(),
        "github.com/team/shared".to_string(),
        vec![
            instance("alpha", "shared", "aaaaaaaa", "aaaaaaa", false, 2),
            instance("beta", "shared", "bbbbbbbb", "bbbbbbb", false, 1),
            instance("gamma", "shared", "cccccccc", "ccccccc", true, 3),
        ],
    );
    let synced = SubrepoStatus::new(
        "stable".to_string(),
        "github.com/team/stable".to_string(),
        vec![
            instance("alpha", "stable", "dddddddd", "ddddddd", false, 1),
            instance("beta", "stable", "dddddddd", "ddddddd", false, 1),
        ],
    );
    let statuses = vec![drifted, synced];

    let summary = generate_status_summary(&statuses, &[], &[], 5, Some(5));
    assert!(summary.contains("repos nested status"));
    assert!(summary.contains("Synced groups     1"));
    assert!(summary.contains("Drifted groups    1"));
    assert!(summary.contains("Shared groups     2"));
    assert!(summary.contains("Nested copies     5"));

    let drift = format_drift_section(&statuses).join("\n");
    assert!(drift.contains("Nested Package Drift"));
    assert!(drift.contains("repos nested sync shared --to aaaaaaa"));
    assert!(drift.contains("alpha/packages/shared"));
    assert!(drift.contains("beta/packages/shared"));
    assert!(drift.contains("gamma/packages/shared"));
    assert!(drift.contains("✓ target"));
    assert!(drift.contains("→ sync"));
    assert!(drift.contains("! dirty"));
}

#[test]
fn nested_drift_is_alphabetical_by_package_then_project() {
    let zeta = SubrepoStatus::new(
        "zeta".to_string(),
        "github.com/team/zeta".to_string(),
        vec![
            instance("zulu-project", "zeta", "bbbbbbbb", "bbbbbbb", false, 2),
            instance("alpha-project", "zeta", "aaaaaaaa", "aaaaaaa", false, 1),
        ],
    );
    let alpha = SubrepoStatus::new(
        "alpha".to_string(),
        "github.com/team/alpha".to_string(),
        vec![
            instance("zulu-project", "alpha", "dddddddd", "ddddddd", false, 2),
            instance("alpha-project", "alpha", "cccccccc", "ccccccc", false, 1),
        ],
    );

    let drift = format_drift_section(&[zeta, alpha]).join("\n");
    let alpha_package = drift.find("pkg:alpha").expect("alpha package");
    let zeta_package = drift.find("pkg:zeta").expect("zeta package");
    assert!(alpha_package < zeta_package, "{drift}");

    let alpha_project = drift
        .find("alpha-project/packages/alpha")
        .expect("alpha project copy");
    let zulu_project = drift
        .find("zulu-project/packages/alpha")
        .expect("zulu project copy");
    assert!(alpha_project < zulu_project, "{drift}");
}

#[test]
fn complete_inventory_distinguishes_groups_copies_and_non_comparable_repositories() {
    let drifted = SubrepoStatus::new(
        "shared".to_string(),
        "github.com/team/shared".to_string(),
        vec![
            instance("alpha", "shared", "aaaaaaaa", "aaaaaaa", false, 2),
            instance("beta", "shared", "bbbbbbbb", "bbbbbbb", false, 1),
        ],
    );
    let unique = SubrepoStatus::new(
        "solo".to_string(),
        "github.com/team/solo".to_string(),
        vec![instance("gamma", "solo", "cccccccc", "ccccccc", false, 1)],
    );
    let missing = instance("delta", "orphan", "dddddddd", "ddddddd", false, 1);
    let report = NestedStatusReport {
        groups: vec![drifted, unique],
        no_remote: vec![missing],
        uninitialized_submodules: Vec::new(),
        total_nested: 4,
        fleet_repositories: 8,
    };

    let summary = generate_status_summary(
        &report.groups,
        &report.no_remote,
        &report.uninitialized_submodules,
        4,
        Some(8),
    );
    assert!(summary.contains("Drifted groups    1"));
    assert!(summary.contains("Shared groups     1"));
    assert!(summary.contains("Unique groups     1"));
    assert!(summary.contains("Missing origin    1"));
    assert!(summary.contains("Nested copies     4"));
    assert!(summary.contains("Fleet repos       8"));
    assert!(summary.contains("Scope: every discovered nested checkout"));
    assert!(summary.contains("Independent       4"));

    let (count, lines) = format_drift_work_items_with_inventory(&report);
    let drift = lines.join("\n");
    assert_eq!(count, 1);
    assert!(drift.contains("1 of 1 shared nested package group is"));
    assert!(drift.contains(
        "Compared 2 shared copies across 8 fleet repositories; 1 unique and 1 missing-origin"
    ));
}

#[test]
fn failed_automatic_drift_check_is_visible() {
    let output = format_drift_failure(&anyhow::anyhow!("nested scan failed")).join("\n");
    assert!(output.contains("Drift check incomplete: nested scan failed"));
    assert!(output.contains("repos nested validate"));
}

#[test]
fn successful_automatic_drift_check_is_visible_even_without_drift() {
    let mut first = instance("alpha", "shared", "aaaaaaaa", "aaaaaaa", false, 1);
    first.checkout_kind = NestedCheckoutKind::Submodule;
    let mut second = instance("beta", "shared", "aaaaaaaa", "aaaaaaa", false, 1);
    second.checkout_kind = NestedCheckoutKind::Submodule;
    let report = NestedStatusReport {
        groups: vec![SubrepoStatus::new(
            "shared".to_string(),
            "github.com/team/shared".to_string(),
            vec![first, second],
        )],
        no_remote: Vec::new(),
        uninitialized_submodules: Vec::new(),
        total_nested: 2,
        fleet_repositories: 4,
    };

    let (count, lines) = format_drift_work_items_with_inventory(&report);
    let output = lines.join("\n");

    assert_eq!(count, 0);
    assert!(output.contains("Nested Package Drift"));
    assert!(output.contains("No commit drift across 1 shared nested package group"));
    assert!(output.contains("0 independent, 2 submodule, 0 linked-worktree copies"));
    assert!(!output.contains("Run `repos nested status`"));
}

#[test]
fn successful_automatic_check_reports_an_empty_nested_inventory() {
    let report = NestedStatusReport {
        groups: Vec::new(),
        no_remote: Vec::new(),
        uninitialized_submodules: Vec::new(),
        total_nested: 0,
        fleet_repositories: 7,
    };

    let (count, lines) = format_drift_work_items_with_inventory(&report);
    let output = lines.join("\n");

    assert_eq!(count, 0);
    assert!(output.contains("No nested checkouts discovered in 7 fleet repositories"));
}

#[test]
fn all_dirty_drift_still_identifies_the_selected_target() {
    let status = SubrepoStatus::new(
        "shared".to_string(),
        "github.com/team/shared".to_string(),
        vec![
            instance("alpha", "shared", "aaaaaaaa", "aaaaaaa", true, 1),
            instance("beta", "shared", "bbbbbbbb", "bbbbbbb", true, 1),
        ],
    );

    let output = format_drift_section(&[status]).join("\n");
    assert!(output.contains("! target"));
    assert!(output.contains("! dirty"));
}

#[test]
fn legacy_status_summary_does_not_invent_complete_fleet_coverage() {
    let statuses = vec![SubrepoStatus::new(
        "shared".to_string(),
        "github.com/team/shared".to_string(),
        vec![
            instance("alpha", "shared", "aaaaaaaa", "aaaaaaa", false, 1),
            instance("beta", "shared", "aaaaaaaa", "aaaaaaa", false, 1),
        ],
    )];

    let summary = generate_status_summary(&statuses, &[], &[], 2, None);
    assert!(summary.contains("Shared copies     2"));
    assert!(summary.contains("complete fleet coverage unavailable"));
    assert!(!summary.contains("Fleet repos"));
    assert!(!summary.contains("Nested copies"));
}

#[test]
fn supplied_fleet_inventory_is_not_replaced_by_a_filesystem_rescan() {
    let report = analyze_nested_status_for_repositories(&[]).unwrap();
    assert_eq!(report.fleet_repositories, 0);
    assert_eq!(report.total_nested, 0);
    assert!(report.groups.is_empty());
    assert!(report.no_remote.is_empty());
}
