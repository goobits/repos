//! Combined two-direction report for `repos sync`.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::git::failure::GitFailure;
use crate::git::Status;
use crate::utils::compare_repository_locations;

use super::{RepositoryOutcome, SyncStatistics};
use crate::core::attention::{append_project_attention_section, AttentionKind, ProjectAttention};
use crate::core::stats::{
    clean_error_message, format_relative_repo_path, truncate_text, BOLD_BLUE, BOLD_PURPLE, DIM,
    GREEN, RED, RESET, YELLOW,
};

#[derive(Default)]
struct SyncRepository {
    path: String,
    pull: Option<RepositoryOutcome>,
    push: Option<RepositoryOutcome>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SyncOutcome {
    Updated,
    UpToDate,
    Skipped,
    Failed,
}

#[derive(Default)]
struct SyncCounts {
    updated: usize,
    up_to_date: usize,
    skipped: usize,
    failed: usize,
}

#[derive(Clone, Copy)]
struct SyncTransferTotals {
    pulled_repos: u64,
    pulled_commits: u64,
    pushed_repos: u64,
    pushed_commits: u64,
}

impl SyncCounts {
    fn add(&mut self, outcome: SyncOutcome) {
        match outcome {
            SyncOutcome::Updated => self.updated += 1,
            SyncOutcome::UpToDate => self.up_to_date += 1,
            SyncOutcome::Skipped => self.skipped += 1,
            SyncOutcome::Failed => self.failed += 1,
        }
    }
}

pub(crate) fn generate_sync_report(
    pull: &SyncStatistics,
    push: &SyncStatistics,
    duration: Duration,
    total_repos: usize,
    show_changes: bool,
    drift_count: usize,
    drift_lines: &[String],
) -> String {
    let repositories = combine_sync_outcomes(pull, push);
    let mut counts = SyncCounts::default();
    for repository in repositories.values() {
        counts.add(sync_outcome(repository));
    }

    let pull_failures = clone_git_failures(pull);
    let push_failures = clone_git_failures(push);
    let local_changes = sync_local_changes(pull, push, &repositories);
    let follow_up_count = local_changes.len() + drift_count;
    let transfers = SyncTransferTotals {
        pulled_repos: pull.pulled_repos.load(Ordering::Relaxed),
        pulled_commits: pull.total_commits_pulled.load(Ordering::Relaxed),
        pushed_repos: push.pushed_repos.load(Ordering::Relaxed),
        pushed_commits: push.total_commits_pushed.load(Ordering::Relaxed),
    };

    let mut lines = vec![
        format!("{BOLD_BLUE}repos sync{RESET}"),
        format!(
            "{GREEN}✓{RESET} Completed in {:.1}s",
            duration.as_secs_f64()
        ),
    ];
    append_transfer_repositories(
        &mut lines,
        "Pulled",
        "↓",
        &clone_transfer_details(&pull.pulled_repo_details),
    );
    append_transfer_repositories(
        &mut lines,
        "Pushed",
        "↑",
        &clone_transfer_details(&push.pushed_repo_details),
    );

    let mut attention = sync_failure_attention(&repositories, &pull_failures, &push_failures);
    attention.extend(sync_skip_attention(&repositories));
    attention.extend(local_changes.into_iter().map(|(repository, path)| {
        ProjectAttention::new(
            AttentionKind::FollowUp,
            repository,
            path,
            "uncommitted changes",
            "commit or stash the local changes",
            None,
        )
    }));
    append_project_attention_section(&mut lines, attention, show_changes);
    if !drift_lines.is_empty() {
        lines.push(String::new());
        lines.extend(drift_lines.iter().cloned());
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }

    lines.push(String::new());
    append_sync_summary(&mut lines, &counts, follow_up_count, total_repos, transfers);
    lines.join("\n")
}

fn append_sync_summary(
    lines: &mut Vec<String>,
    counts: &SyncCounts,
    follow_up_count: usize,
    total_repos: usize,
    transfers: SyncTransferTotals,
) {
    lines.push(format!("{BOLD_PURPLE}▌ Summary{RESET}"));
    lines.push(format!(
        "  {GREEN}✓{RESET} {:<16}{}",
        "Updated", counts.updated
    ));
    lines.push(format!(
        "  {GREEN}✓{RESET} {:<16}{}",
        "Up to date", counts.up_to_date
    ));
    if counts.failed > 0 {
        lines.push(format!("  {RED}!{RESET} {:<16}{}", "Failed", counts.failed));
    }
    if counts.skipped > 0 {
        lines.push(format!(
            "  {DIM}·{RESET} {:<16}{}",
            "Skipped", counts.skipped
        ));
    }
    if follow_up_count > 0 {
        lines.push(format!(
            "  {YELLOW}~{RESET} {:<16}{follow_up_count}",
            "Follow-up"
        ));
    }
    lines.push(format!(
        "  {GREEN}↓{RESET} {:<16}{} {} / {} {}",
        "Pulled",
        transfers.pulled_repos,
        plural(transfers.pulled_repos, "repo", "repos"),
        transfers.pulled_commits,
        plural(transfers.pulled_commits, "commit", "commits")
    ));
    lines.push(format!(
        "  {GREEN}↑{RESET} {:<16}{} {} / {} {}",
        "Pushed",
        transfers.pushed_repos,
        plural(transfers.pushed_repos, "repo", "repos"),
        transfers.pushed_commits,
        plural(transfers.pushed_commits, "commit", "commits")
    ));
    lines.push(format!("  {DIM}·{RESET} {:<16}{total_repos}", "Checked"));
}

fn combine_sync_outcomes(
    pull: &SyncStatistics,
    push: &SyncStatistics,
) -> BTreeMap<String, SyncRepository> {
    let mut repositories = BTreeMap::<String, SyncRepository>::new();
    for outcome in pull.batch_outcomes() {
        let repository = repositories.entry(outcome.repository.clone()).or_default();
        repository.path.clone_from(&outcome.path);
        repository.pull = Some(outcome);
    }
    for outcome in push.batch_outcomes() {
        let repository = repositories.entry(outcome.repository.clone()).or_default();
        repository.path.clone_from(&outcome.path);
        repository.push = Some(outcome);
    }
    repositories
}

fn sync_outcome(repository: &SyncRepository) -> SyncOutcome {
    let phases = [repository.pull.as_ref(), repository.push.as_ref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if phases.iter().any(|outcome| is_failure(outcome.status)) {
        SyncOutcome::Failed
    } else if phases.iter().any(|outcome| {
        matches!(
            outcome.status,
            Status::Pulled | Status::Pushed | Status::Fetched
        )
    }) {
        SyncOutcome::Updated
    } else if phases.iter().any(|outcome| is_skip(outcome.status)) {
        SyncOutcome::Skipped
    } else {
        SyncOutcome::UpToDate
    }
}

fn is_failure(status: Status) -> bool {
    matches!(
        status,
        Status::Error
            | Status::ConfigError
            | Status::StagingError
            | Status::CommitError
            | Status::PullError
    )
}

fn is_skip(status: Status) -> bool {
    matches!(
        status,
        Status::Skip
            | Status::NoUpstream
            | Status::NoRemote
            | Status::ConfigSkipped
            | Status::NoChanges
            | Status::Dirty
    )
}

fn clone_git_failures(statistics: &SyncStatistics) -> HashMap<(String, String), GitFailure> {
    statistics
        .git_failures
        .lock()
        .map(|failures| failures.clone())
        .unwrap_or_default()
}

fn clone_transfer_details(
    details: &std::sync::Mutex<Vec<(String, String, u64)>>,
) -> Vec<(String, String, u64)> {
    let mut details = details
        .lock()
        .map(|details| details.clone())
        .unwrap_or_default();
    details
        .sort_by(|left, right| compare_repository_locations(&left.1, &left.0, &right.1, &right.0));
    details
}

fn append_transfer_repositories(
    lines: &mut Vec<String>,
    heading: &str,
    marker: &str,
    repositories: &[(String, String, u64)],
) {
    if repositories.is_empty() {
        return;
    }
    lines.push(String::new());
    lines.push(format!("{BOLD_PURPLE}▌ {heading}{RESET}"));
    for (repository, path, commits) in repositories {
        lines.push(format!(
            "  {GREEN}{marker}{RESET} {:24} {commits} {}",
            truncate_text(repository, 24),
            plural(*commits, "commit", "commits")
        ));
        lines.push(format!(
            "    {DIM}↳ path: {}{RESET}",
            format_relative_repo_path(path)
        ));
    }
}

fn sync_failure_attention(
    repositories: &BTreeMap<String, SyncRepository>,
    pull_failures: &HashMap<(String, String), GitFailure>,
    push_failures: &HashMap<(String, String), GitFailure>,
) -> Vec<ProjectAttention> {
    repositories
        .iter()
        .filter(|(_, repository)| sync_outcome(repository) == SyncOutcome::Failed)
        .map(|(name, repository)| {
            let display_path = format_relative_repo_path(&repository.path);
            let pull_failure = repository
                .pull
                .as_ref()
                .filter(|outcome| is_failure(outcome.status));
            let push_failure = repository
                .push
                .as_ref()
                .filter(|outcome| is_failure(outcome.status));
            let mut reasons = Vec::new();
            let mut structured_failure = None;
            if let Some(outcome) = pull_failure {
                let failure = pull_failures.get(&(name.clone(), outcome.path.clone()));
                reasons.push(format!(
                    "pull: {}",
                    failure
                        .map_or_else(|| clean_error_message(&outcome.message), GitFailure::reason)
                ));
                structured_failure = failure;
            }
            if let Some(outcome) = push_failure {
                let failure = push_failures.get(&(name.clone(), outcome.path.clone()));
                reasons.push(format!(
                    "push: {}",
                    failure
                        .map_or_else(|| clean_error_message(&outcome.message), GitFailure::reason)
                ));
                structured_failure = failure.or(structured_failure);
            }

            let next = structured_failure.map_or_else(
                || "run `repos doctor`, then inspect this repository".to_string(),
                |failure| failure.next_action(&display_path),
            );
            let remote = structured_failure
                .and_then(|failure| failure.remote.as_ref())
                .map(|remote| remote.display());
            ProjectAttention::new(
                AttentionKind::Failed,
                name.clone(),
                repository.path.clone(),
                reasons.join("; "),
                next,
                remote,
            )
        })
        .collect()
}

fn sync_skip_attention(repositories: &BTreeMap<String, SyncRepository>) -> Vec<ProjectAttention> {
    repositories
        .iter()
        .filter(|(_, repository)| sync_outcome(repository) == SyncOutcome::Skipped)
        .map(|(name, repository)| {
            let mut reasons = Vec::new();
            let mut statuses = Vec::new();
            if let Some(outcome) = repository
                .pull
                .as_ref()
                .filter(|outcome| is_skip(outcome.status))
            {
                reasons.push(format!("pull: {}", clean_error_message(&outcome.message)));
                statuses.push(("pull", outcome));
            }
            if let Some(outcome) = repository
                .push
                .as_ref()
                .filter(|outcome| is_skip(outcome.status))
            {
                reasons.push(format!("push: {}", clean_error_message(&outcome.message)));
                statuses.push(("push", outcome));
            }
            ProjectAttention::new(
                AttentionKind::Skipped,
                name.clone(),
                repository.path.clone(),
                reasons.join("; "),
                sync_skip_next(&statuses),
                None,
            )
        })
        .collect()
}

fn sync_skip_next(statuses: &[(&str, &RepositoryOutcome)]) -> &'static str {
    if statuses
        .iter()
        .any(|(_, outcome)| matches!(outcome.status, Status::NoRemote))
    {
        "add a remote or exclude this repository"
    } else if statuses
        .iter()
        .any(|(phase, outcome)| *phase == "push" && matches!(outcome.status, Status::NoUpstream))
    {
        "run `repos push --auto-upstream`"
    } else if statuses
        .iter()
        .any(|(_, outcome)| matches!(outcome.status, Status::NoUpstream))
    {
        "set an upstream branch or exclude this repository"
    } else if statuses
        .iter()
        .any(|(_, outcome)| matches!(outcome.status, Status::Dirty))
    {
        "commit or stash local changes, then rerun `repos sync`"
    } else if statuses.iter().any(|(_, outcome)| {
        matches!(outcome.status, Status::Skip) && outcome.message.contains("detached HEAD")
    }) {
        "checkout a branch"
    } else {
        "run `repos status --skipped`"
    }
}

fn sync_local_changes(
    pull: &SyncStatistics,
    push: &SyncStatistics,
    repositories: &BTreeMap<String, SyncRepository>,
) -> Vec<(String, String)> {
    let mut local = BTreeMap::new();
    for statistics in [pull, push] {
        if let Ok(changes) = statistics.uncommitted_repos.lock() {
            for (name, path) in changes.iter() {
                let is_follow_up = repositories.get(name).is_some_and(|repository| {
                    matches!(
                        sync_outcome(repository),
                        SyncOutcome::Updated | SyncOutcome::UpToDate
                    )
                });
                if is_follow_up {
                    local.entry(name.clone()).or_insert_with(|| path.clone());
                }
            }
        }
    }
    local.into_iter().collect()
}

fn plural(count: u64, singular: &'static str, plural: &'static str) -> &'static str {
    if count == 1 {
        singular
    } else {
        plural
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(status: Status, message: &str) -> RepositoryOutcome {
        RepositoryOutcome {
            repository: "app".to_string(),
            path: "./app".to_string(),
            status,
            message: message.to_string(),
            has_uncommitted: false,
        }
    }

    #[test]
    fn successful_push_takes_precedence_over_pull_skip() {
        let repository = SyncRepository {
            path: "./app".to_string(),
            pull: Some(outcome(Status::NoUpstream, "no tracking")),
            push: Some(outcome(Status::Pushed, "set upstream & pushed")),
        };

        assert_eq!(sync_outcome(&repository), SyncOutcome::Updated);
    }

    #[test]
    fn successful_auto_upstream_push_removes_stale_skip_guidance() {
        let pull = SyncStatistics::new();
        pull.update("app", "./app", &Status::NoUpstream, "no tracking", false);
        let push = SyncStatistics::new();
        push.update(
            "app",
            "./app",
            &Status::Pushed,
            "set upstream & pushed",
            false,
        );

        let report = generate_sync_report(&pull, &push, Duration::ZERO, 1, false, 0, &[]);

        assert!(report.contains("Updated         1"), "{report}");
        assert!(!report.contains("Skipped         1"), "{report}");
        assert!(
            !report.contains("run `repos push --auto-upstream`"),
            "{report}"
        );
    }

    #[test]
    fn failure_still_takes_precedence_over_successful_transfer() {
        let repository = SyncRepository {
            path: "./app".to_string(),
            pull: Some(outcome(Status::PullError, "pull failed")),
            push: Some(outcome(Status::Pushed, "pushed")),
        };

        assert_eq!(sync_outcome(&repository), SyncOutcome::Failed);
    }
}
