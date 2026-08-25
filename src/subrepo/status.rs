//! Nested-repository status models and inventory analysis.
//!
//! Formatting and terminal rendering live in focused child modules so the
//! domain model does not depend on presentation details.

mod detail;
mod display;
mod format;
mod style;

#[cfg(test)]
mod tests;

use super::SubrepoInstance;
use anyhow::Result;
use std::collections::HashSet;
use std::path::PathBuf;

pub use display::{display_nested_status, display_status};
pub use format::{
    display_drift_summary, format_drift_failure, format_drift_section, format_drift_work_items,
    format_drift_work_items_with_inventory,
};

/// Status of one remote-backed nested repository across its fleet copies.
#[derive(Debug)]
pub struct SubrepoStatus {
    pub name: String,
    pub remote_url: String,
    pub instances: Vec<SubrepoInstance>,
    pub sync_score: f32,
    pub unique_commits: usize,
    pub has_drift: bool,
}

/// Complete status inventory for independent nested repositories.
#[derive(Debug)]
pub struct NestedStatusReport {
    pub groups: Vec<SubrepoStatus>,
    pub no_remote: Vec<SubrepoInstance>,
    pub total_nested: usize,
    pub fleet_repositories: usize,
}

impl NestedStatusReport {
    pub fn shared_groups(&self) -> impl Iterator<Item = &SubrepoStatus> {
        self.groups
            .iter()
            .filter(|status| status.instances.len() > 1)
    }

    pub fn unique_groups(&self) -> impl Iterator<Item = &SubrepoStatus> {
        self.groups
            .iter()
            .filter(|status| status.instances.len() == 1)
    }

    #[must_use]
    pub fn drifted_count(&self) -> usize {
        self.shared_groups()
            .filter(|status| status.has_drift)
            .count()
    }

    #[must_use]
    pub fn synced_count(&self) -> usize {
        self.shared_groups()
            .filter(|status| !status.has_drift)
            .count()
    }

    #[must_use]
    pub fn shared_group_count(&self) -> usize {
        self.shared_groups().count()
    }

    #[must_use]
    pub fn shared_copy_count(&self) -> usize {
        self.shared_groups()
            .map(|status| status.instances.len())
            .sum()
    }
}

impl SubrepoStatus {
    /// Calculate sync score: (`total_instances` - `unique_commits`) / (`total_instances` - 1) × 100.
    fn calculate_sync_score(instances: &[SubrepoInstance]) -> (f32, usize) {
        let unique_commits = instances
            .iter()
            .map(|instance| &instance.commit_hash)
            .collect::<HashSet<_>>()
            .len();

        if instances.len() <= 1 {
            return (100.0, unique_commits);
        }

        let score =
            ((instances.len() - unique_commits) as f32) / ((instances.len() - 1) as f32) * 100.0;
        (score, unique_commits)
    }

    /// Create a status from every copy that shares one normalized remote.
    #[must_use]
    pub fn new(name: String, remote_url: String, instances: Vec<SubrepoInstance>) -> Self {
        let (sync_score, unique_commits) = Self::calculate_sync_score(&instances);
        let has_drift = unique_commits > 1;

        Self {
            name,
            remote_url,
            instances,
            sync_score,
            unique_commits,
            has_drift,
        }
    }
}

/// Analyze all nested repositories, including unique and missing-origin copies.
pub fn analyze_nested_status() -> Result<NestedStatusReport> {
    let (report, fleet_repositories) = super::validation::validate_subrepos_inventory(true)?;
    Ok(analyze_nested_status_from_report(
        report,
        fleet_repositories,
    ))
}

/// Analyze all nested repositories without printing scan progress.
pub fn analyze_nested_status_quiet() -> Result<NestedStatusReport> {
    let (report, fleet_repositories) = super::validation::validate_subrepos_inventory(false)?;
    Ok(analyze_nested_status_from_report(
        report,
        fleet_repositories,
    ))
}

/// Analyze an existing fleet discovery snapshot without rescanning the filesystem.
pub(crate) fn analyze_nested_status_for_repositories(
    repositories: &[(String, PathBuf)],
) -> Result<NestedStatusReport> {
    let report = super::validation::validate_discovered_repositories(repositories, false)?;
    Ok(analyze_nested_status_from_report(
        report,
        repositories.len(),
    ))
}

/// Analyze all subrepos and return status for shared ones.
///
/// This compatibility helper retains the original command-plumbing API. New
/// report code should use [`analyze_nested_status`] so unique and missing-origin
/// repositories cannot disappear from its coverage accounting.
pub fn analyze_subrepos() -> Result<Vec<SubrepoStatus>> {
    Ok(analyze_nested_status()?
        .groups
        .into_iter()
        .filter(|status| status.instances.len() > 1)
        .collect())
}

/// Analyze shared subrepos without printing scan progress.
pub fn analyze_subrepos_quiet() -> Result<Vec<SubrepoStatus>> {
    Ok(analyze_nested_status_quiet()?
        .groups
        .into_iter()
        .filter(|status| status.instances.len() > 1)
        .collect())
}

fn analyze_nested_status_from_report(
    report: super::ValidationReport,
    fleet_repositories: usize,
) -> NestedStatusReport {
    let mut statuses = report
        .by_remote
        .into_iter()
        .filter_map(|(remote_url, instances)| {
            let name = instances.first()?.subrepo_name.clone();
            Some(SubrepoStatus::new(name, remote_url, instances))
        })
        .collect::<Vec<_>>();

    statuses.sort_by(|left, right| {
        left.sync_score
            .total_cmp(&right.sync_score)
            .then_with(|| compare_package_statuses(left, right))
    });

    NestedStatusReport {
        groups: statuses,
        no_remote: report.no_remote,
        total_nested: report.total_nested,
        fleet_repositories,
    }
}

fn compare_package_statuses(left: &SubrepoStatus, right: &SubrepoStatus) -> std::cmp::Ordering {
    left.name
        .cmp(&right.name)
        .then_with(|| left.remote_url.cmp(&right.remote_url))
}

fn compare_target_instances(left: &SubrepoInstance, right: &SubrepoInstance) -> std::cmp::Ordering {
    left.commit_timestamp
        .cmp(&right.commit_timestamp)
        .then_with(|| left.commit_hash.cmp(&right.commit_hash))
        .then_with(|| instance_location(left).cmp(&instance_location(right)))
}

fn select_sync_target(status: &SubrepoStatus) -> Option<&SubrepoInstance> {
    status
        .instances
        .iter()
        .filter(|instance| !instance.has_uncommitted)
        .max_by(|left, right| compare_target_instances(left, right))
        .or_else(|| {
            status
                .instances
                .iter()
                .max_by(|left, right| compare_target_instances(left, right))
        })
}

fn instance_location(instance: &SubrepoInstance) -> String {
    match instance.relative_path.as_str() {
        "" | "." => instance.parent_repo.clone(),
        relative_path => format!("{}/{}", instance.parent_repo, relative_path),
    }
}
