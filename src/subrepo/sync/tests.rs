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
        uninitialized_submodules: Vec::new(),
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
