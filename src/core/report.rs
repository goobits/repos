//! Shared reporting for fleet-wide repository mutations.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::git::failure::GitFailure;
use crate::git::Status;
use crate::utils::compare_repository_locations;

use super::attention::{append_project_attention_section, AttentionKind, ProjectAttention};
use super::stats::{
    clean_error_message, format_relative_repo_path, truncate_text, SyncStatistics, BOLD_BLUE,
    BOLD_PURPLE, DIM, GREEN, RED, RESET, YELLOW,
};

#[derive(Clone, Debug)]
pub(crate) struct RepositoryOutcome {
    pub repository: String,
    pub path: String,
    pub status: Status,
    pub message: String,
    pub has_uncommitted: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BatchOperation {
    Save { dry_run: bool },
    Stage,
    Unstage,
    Commit,
    Config { dry_run: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutcomeKind {
    Changed,
    Unchanged,
    Planned,
    NeedsWork,
    Skipped,
    Failed,
}

struct Section<'a> {
    heading: &'a str,
    kind: OutcomeKind,
    color: &'a str,
    marker: &'a str,
    actionable: bool,
}

#[derive(Default)]
struct OutcomeCounts {
    changed: usize,
    unchanged: usize,
    planned: usize,
    needs_work: usize,
    skipped: usize,
    failed: usize,
}

impl OutcomeCounts {
    fn add(&mut self, kind: OutcomeKind) {
        match kind {
            OutcomeKind::Changed => self.changed += 1,
            OutcomeKind::Unchanged => self.unchanged += 1,
            OutcomeKind::Planned => self.planned += 1,
            OutcomeKind::NeedsWork => self.needs_work += 1,
            OutcomeKind::Skipped => self.skipped += 1,
            OutcomeKind::Failed => self.failed += 1,
        }
    }

    fn processed(&self) -> usize {
        self.changed + self.unchanged + self.planned + self.needs_work + self.skipped + self.failed
    }
}

impl BatchOperation {
    fn command(self) -> &'static str {
        match self {
            Self::Save { .. } => "save",
            Self::Stage => "stage",
            Self::Unstage => "unstage",
            Self::Commit => "commit",
            Self::Config { .. } => "config",
        }
    }

    fn changed_label(self) -> &'static str {
        match self {
            Self::Save { .. } => "Saved",
            Self::Stage => "Staged",
            Self::Unstage => "Unstaged",
            Self::Commit => "Committed",
            Self::Config { .. } => "Updated",
        }
    }

    fn unchanged_label(self) -> &'static str {
        match self {
            Self::Save { .. } => "Clean",
            Self::Stage | Self::Unstage => "No match",
            Self::Commit => "Nothing staged",
            Self::Config { .. } => "Already correct",
        }
    }

    fn classify(self, outcome: &RepositoryOutcome) -> OutcomeKind {
        match outcome.status {
            Status::Error
            | Status::ConfigError
            | Status::StagingError
            | Status::CommitError
            | Status::PullError => OutcomeKind::Failed,
            Status::NoRemote | Status::NoUpstream | Status::Dirty => OutcomeKind::NeedsWork,
            Status::Skip => OutcomeKind::Skipped,
            Status::ConfigSkipped
                if matches!(self, Self::Config { dry_run: true })
                    && outcome.message.starts_with("would update:") =>
            {
                OutcomeKind::Planned
            }
            Status::ConfigSkipped => OutcomeKind::Skipped,
            Status::NoChanges
                if matches!(self, Self::Save { .. })
                    && outcome.message.contains("only untracked changes") =>
            {
                OutcomeKind::NeedsWork
            }
            Status::NoChanges => OutcomeKind::Unchanged,
            Status::Staged if matches!(self, Self::Save { dry_run: true }) => OutcomeKind::Planned,
            Status::Synced | Status::ConfigSynced => OutcomeKind::Unchanged,
            Status::Pushed
            | Status::Pulled
            | Status::Fetched
            | Status::ConfigUpdated
            | Status::Staged
            | Status::Unstaged
            | Status::Committed => OutcomeKind::Changed,
        }
    }

    fn next_action(self, outcome: &RepositoryOutcome) -> &'static str {
        match outcome.status {
            Status::NoRemote => "add a remote or exclude this repository",
            Status::NoUpstream if matches!(self, Self::Save { .. }) => {
                "run `repos push --auto-upstream`"
            }
            Status::NoUpstream => "set an upstream branch or exclude this repository",
            Status::Dirty => "commit or stash the local changes",
            Status::Skip if outcome.message.contains("detached HEAD") => "checkout a branch",
            Status::ConfigSkipped => "rerun and approve the update, or pass `--yes`",
            Status::NoChanges if matches!(self, Self::Save { .. }) => {
                "pass `--include-untracked` if those files should be saved"
            }
            Status::Error
            | Status::ConfigError
            | Status::StagingError
            | Status::CommitError
            | Status::PullError => "inspect the reported failure",
            _ => "inspect this repository",
        }
    }
}

impl SyncStatistics {
    /// Generates a compact live footer for non-transfer fleet operations.
    pub(crate) fn generate_batch_live_summary(
        &self,
        operation: BatchOperation,
        total_repos: usize,
    ) -> String {
        let outcomes = self.batch_outcomes();
        let counts = summarize(operation, &outcomes);
        let remaining = total_repos.saturating_sub(counts.processed());
        let action = operation.changed_label().to_lowercase();
        let unchanged = operation.unchanged_label().to_lowercase();

        format!(
            "  {GREEN}✓{RESET} {} {action}   {GREEN}✓{RESET} {} {unchanged}   {YELLOW}~{RESET} {} planned   {RED}!{RESET} {} failed   {YELLOW}!{RESET} {} needs work   {DIM}·{RESET} {} skipped\n  {DIM}↳ scanning {remaining} remaining{RESET}",
            counts.changed,
            counts.unchanged,
            counts.planned,
            counts.failed,
            counts.needs_work,
            counts.skipped,
        )
    }

    /// Generates the stable final report for non-transfer fleet operations.
    pub(crate) fn generate_batch_report(
        &self,
        operation: BatchOperation,
        duration: Duration,
    ) -> String {
        let mut outcomes = self.batch_outcomes();
        outcomes.sort_by(|left, right| {
            compare_repository_locations(
                &left.path,
                &left.repository,
                &right.path,
                &right.repository,
            )
        });
        let counts = summarize(operation, &outcomes);
        let mut lines = vec![
            format!("{BOLD_BLUE}repos {}{RESET}", operation.command()),
            format!(
                "{GREEN}✓{RESET} Completed in {:.1}s",
                duration.as_secs_f64()
            ),
            String::new(),
            format!("{BOLD_PURPLE}▌ Summary{RESET}"),
            format!(
                "  {GREEN}✓{RESET} {:<16}{}",
                operation.changed_label(),
                counts.changed
            ),
            format!(
                "  {GREEN}✓{RESET} {:<16}{}",
                operation.unchanged_label(),
                counts.unchanged
            ),
        ];

        if counts.planned > 0 {
            lines.push(format!(
                "  {YELLOW}~{RESET} {:<16}{}",
                "Planned", counts.planned
            ));
        }
        if counts.failed > 0 {
            lines.push(format!("  {RED}!{RESET} {:<16}{}", "Failed", counts.failed));
        }
        if counts.needs_work > 0 {
            lines.push(format!(
                "  {YELLOW}!{RESET} {:<16}{}",
                "Needs work", counts.needs_work
            ));
        }
        if counts.skipped > 0 {
            lines.push(format!(
                "  {DIM}·{RESET} {:<16}{}",
                "Skipped", counts.skipped
            ));
        }
        lines.push(format!(
            "  {DIM}·{RESET} {:<16}{}",
            "Checked",
            counts.processed()
        ));

        let sections = [
            Section {
                heading: operation.changed_label(),
                kind: OutcomeKind::Changed,
                color: GREEN,
                marker: "✓",
                actionable: false,
            },
            Section {
                heading: "Planned",
                kind: OutcomeKind::Planned,
                color: YELLOW,
                marker: "~",
                actionable: false,
            },
            Section {
                heading: "Failed",
                kind: OutcomeKind::Failed,
                color: RED,
                marker: "!",
                actionable: true,
            },
            Section {
                heading: "Needs Work",
                kind: OutcomeKind::NeedsWork,
                color: YELLOW,
                marker: "!",
                actionable: true,
            },
            Section {
                heading: "Skipped",
                kind: OutcomeKind::Skipped,
                color: DIM,
                marker: "·",
                actionable: true,
            },
        ];
        for section in sections {
            append_section(&mut lines, operation, section, &outcomes);
        }

        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }

    pub(super) fn batch_outcomes(&self) -> Vec<RepositoryOutcome> {
        match self.operation_outcomes.lock() {
            Ok(outcomes) => outcomes.clone(),
            Err(_) => {
                eprintln!("Warning: Failed to acquire lock for operation outcomes");
                Vec::new()
            }
        }
    }
}

fn summarize(operation: BatchOperation, outcomes: &[RepositoryOutcome]) -> OutcomeCounts {
    let mut counts = OutcomeCounts::default();
    for outcome in outcomes {
        counts.add(operation.classify(outcome));
    }
    counts
}

fn append_section(
    lines: &mut Vec<String>,
    operation: BatchOperation,
    section: Section<'_>,
    outcomes: &[RepositoryOutcome],
) {
    let matching = outcomes
        .iter()
        .filter(|outcome| operation.classify(outcome) == section.kind)
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return;
    }

    lines.push(String::new());
    lines.push(format!("{BOLD_PURPLE}▌ {}{RESET}", section.heading));
    for outcome in matching {
        let message = clean_error_message(&outcome.message);
        lines.push(format!(
            "  {}{}{RESET} {:24} {message}",
            section.color,
            section.marker,
            truncate_text(&outcome.repository, 24)
        ));
        lines.push(format!("    ↳ path: {}", outcome.path));
        if section.actionable {
            lines.push(format!("    ↳ next: {}", operation.next_action(outcome)));
        }
    }
}

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

/// Formats a single final report for the two directional phases of `repos sync`.
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
    let pulled_repos = pull.pulled_repos.load(Ordering::Relaxed);
    let pulled_commits = pull.total_commits_pulled.load(Ordering::Relaxed);
    let pushed_repos = push.pushed_repos.load(Ordering::Relaxed);
    let pushed_commits = push.total_commits_pushed.load(Ordering::Relaxed);
    let transfers = SyncTransferTotals {
        pulled_repos,
        pulled_commits,
        pushed_repos,
        pushed_commits,
    };

    let mut lines = vec![
        format!("{BOLD_BLUE}repos sync{RESET}"),
        format!(
            "{GREEN}✓{RESET} Completed in {:.1}s",
            duration.as_secs_f64()
        ),
        String::new(),
    ];
    append_sync_totals(
        &mut lines,
        "Summary",
        &counts,
        follow_up_count,
        total_repos,
        None,
    );
    lines.push(String::new());
    lines.push(format!("{BOLD_PURPLE}▌ Transfers{RESET}"));
    lines.push(format!(
        "  {GREEN}↓{RESET} {:<16}{pulled_repos} {} / {pulled_commits} {}",
        "Pulled",
        plural(pulled_repos, "repo", "repos"),
        plural(pulled_commits, "commit", "commits")
    ));
    lines.push(format!(
        "  {GREEN}↑{RESET} {:<16}{pushed_repos} {} / {pushed_commits} {}",
        "Pushed",
        plural(pushed_repos, "repo", "repos"),
        plural(pushed_commits, "commit", "commits")
    ));

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
    append_sync_totals(
        &mut lines,
        "Final Totals",
        &counts,
        follow_up_count,
        total_repos,
        Some(transfers),
    );
    lines.join("\n")
}

fn append_sync_totals(
    lines: &mut Vec<String>,
    heading: &str,
    counts: &SyncCounts,
    follow_up_count: usize,
    total_repos: usize,
    transfers: Option<SyncTransferTotals>,
) {
    lines.push(format!("{BOLD_PURPLE}▌ {heading}{RESET}"));
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
    if let Some(transfers) = transfers {
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
    }
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
    } else if phases.iter().any(|outcome| is_skip(outcome.status)) {
        SyncOutcome::Skipped
    } else if phases.iter().any(|outcome| {
        matches!(
            outcome.status,
            Status::Pulled | Status::Pushed | Status::Fetched
        )
    }) {
        SyncOutcome::Updated
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
                let is_standalone_follow_up = repositories.get(name).is_some_and(|repository| {
                    matches!(
                        sync_outcome(repository),
                        SyncOutcome::Updated | SyncOutcome::UpToDate
                    )
                });
                if is_standalone_follow_up {
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

    #[test]
    fn save_report_names_mutations_and_actionable_non_successes() {
        let stats = SyncStatistics::new();
        stats.update(
            "saved-app",
            "./saved-app",
            &Status::Committed,
            "committed; pushed 1 commit",
            false,
        );
        stats.update(
            "detached-app",
            "./detached-app",
            &Status::Skip,
            "detached HEAD; checkout a branch before save",
            false,
        );

        let report = stats.generate_batch_report(
            BatchOperation::Save { dry_run: false },
            Duration::from_secs(2),
        );

        assert!(report.contains("repos save"));
        assert!(report.contains("▌ Saved"));
        assert!(report.contains("saved-app"));
        assert!(report.contains("▌ Skipped"));
        assert!(report.contains("path: ./detached-app"));
        assert!(report.contains("next: checkout a branch"));
        assert!(!report.contains("Pushed"));
    }

    #[test]
    fn save_report_flags_untracked_only_repositories_with_an_exact_next_step() {
        let stats = SyncStatistics::new();
        stats.update(
            "scratch-app",
            "./scratch-app",
            &Status::NoChanges,
            "only untracked changes; pass --include-untracked",
            true,
        );

        let report =
            stats.generate_batch_report(BatchOperation::Save { dry_run: false }, Duration::ZERO);

        assert!(report.contains("Needs work      1"));
        assert!(report.contains("path: ./scratch-app"));
        assert!(report.contains("next: pass `--include-untracked`"));
    }

    #[test]
    fn stage_and_unstage_reports_use_their_own_labels() {
        let stats = SyncStatistics::new();
        stats.update("web", "./web", &Status::Staged, "staged *.rs", false);
        let stage = stats.generate_batch_report(BatchOperation::Stage, Duration::ZERO);
        assert!(stage.contains("repos stage"));
        assert!(stage.contains("Staged"));

        let stats = SyncStatistics::new();
        stats.update("web", "./web", &Status::Unstaged, "unstaged *.rs", false);
        let unstage = stats.generate_batch_report(BatchOperation::Unstage, Duration::ZERO);
        assert!(unstage.contains("repos unstage"));
        assert!(unstage.contains("Unstaged"));
    }

    #[test]
    fn commit_report_distinguishes_committed_from_nothing_staged() {
        let stats = SyncStatistics::new();
        stats.update(
            "changed",
            "./changed",
            &Status::Committed,
            "committed abc1234",
            false,
        );
        stats.update(
            "idle",
            "./idle",
            &Status::NoChanges,
            "nothing to commit",
            false,
        );

        let report = stats.generate_batch_report(BatchOperation::Commit, Duration::ZERO);
        assert!(report.contains("Committed       1"));
        assert!(report.contains("Nothing staged  1"));
        assert!(report.contains("changed"));
        assert!(!report.contains("Pushed"));
    }

    #[test]
    fn config_dry_run_lists_planned_repositories() {
        let stats = SyncStatistics::new();
        stats.update(
            "api",
            "./api",
            &Status::ConfigSkipped,
            "would update: email → developer@example.com",
            false,
        );

        let report =
            stats.generate_batch_report(BatchOperation::Config { dry_run: true }, Duration::ZERO);
        assert!(report.contains("repos config"));
        assert!(report.contains("Planned         1"));
        assert!(report.contains("api"));
        assert!(!report.contains("Pushed"));
    }

    #[test]
    fn batch_report_groups_results_by_project_path() {
        let stats = SyncStatistics::new();
        stats.update(
            "a-package",
            "./zeta/packages/a",
            &Status::StagingError,
            "failed",
            false,
        );
        stats.update(
            "z-package",
            "./alpha/packages/z",
            &Status::StagingError,
            "failed",
            false,
        );

        let report = stats.generate_batch_report(BatchOperation::Stage, Duration::ZERO);

        let alpha = report
            .find("path: ./alpha/packages/z")
            .expect("alpha project should be reported");
        let zeta = report
            .find("path: ./zeta/packages/a")
            .expect("zeta project should be reported");
        assert!(alpha < zeta, "{report}");
    }

    #[test]
    fn sync_report_combines_both_phases_into_exclusive_repo_outcomes() {
        let pull = SyncStatistics::new();
        pull.update("current", "./current", &Status::Synced, "up to date", true);
        pull.update(
            "incoming",
            "./incoming",
            &Status::Pulled,
            "2 commits pulled",
            false,
        );
        pull.update(
            "outgoing",
            "./outgoing",
            &Status::Synced,
            "up to date",
            false,
        );
        pull.update(
            "missing",
            "./missing",
            &Status::NoRemote,
            "no remote configured",
            false,
        );
        pull.update(
            "broken",
            "./broken",
            &Status::PullError,
            "network unavailable",
            false,
        );

        let push = SyncStatistics::new();
        push.update("current", "./current", &Status::Synced, "up to date", true);
        push.update(
            "incoming",
            "./incoming",
            &Status::Synced,
            "up to date",
            false,
        );
        push.update(
            "outgoing",
            "./outgoing",
            &Status::Pushed,
            "1 commit pushed",
            false,
        );
        push.update(
            "missing",
            "./missing",
            &Status::NoRemote,
            "no remote configured",
            false,
        );
        push.update("broken", "./broken", &Status::Synced, "up to date", false);

        let report = generate_sync_report(&pull, &push, Duration::from_secs(4), 5, false, 0, &[]);

        assert_eq!(report.matches("repos sync").count(), 1);
        assert!(!report.contains("repos pull"));
        assert!(!report.contains("repos push\x1b"));
        assert!(report.contains("Updated         2"));
        assert!(report.contains("Up to date      1"));
        assert!(report.contains("Failed          1"));
        assert!(report.contains("Skipped         1"));
        assert!(report.contains("Checked         5"));
        assert!(report.contains("▌ Pulled"));
        assert!(report.contains("incoming"));
        assert!(report.contains("▌ Pushed"));
        assert!(report.contains("outgoing"));
        assert!(report.contains("path: ./broken"));
        assert!(report.contains("path: ./missing"));
        assert!(report.contains("Follow-up       1"));
        assert!(report.contains("uncommitted changes"));
        assert_eq!(report.matches("▌ Needs Attention by Project").count(), 1);
        assert!(!report.contains("▌ Failed"));
        assert!(!report.contains("▌ Skipped"));
        assert!(!report.contains("▌ Follow-up"));
        let attention_index = report
            .rfind("▌ Needs Attention by Project")
            .expect("attention section should be present");
        let totals_index = report
            .rfind("▌ Final Totals")
            .expect("final totals should be present");
        assert!(totals_index > attention_index, "{report}");
        assert!(report.contains("Pulled          1 repo / 2 commits"));
        assert!(report.contains("Pushed          1 repo / 1 commit"));
        assert!(report.trim_end().ends_with("Checked         5"), "{report}");
    }

    #[test]
    fn sync_report_groups_each_repository_section_by_project_path() {
        let pull = SyncStatistics::new();
        for (name, path, status, message, has_uncommitted) in [
            (
                "a-pulled",
                "./zeta/apps/pulled",
                Status::Pulled,
                "1 commit pulled",
                false,
            ),
            (
                "z-pulled",
                "./alpha/apps/pulled",
                Status::Pulled,
                "1 commit pulled",
                false,
            ),
            (
                "a-failed",
                "./zeta/packages/failed",
                Status::PullError,
                "network unavailable",
                false,
            ),
            (
                "z-failed",
                "./alpha/packages/failed",
                Status::PullError,
                "network unavailable",
                false,
            ),
            (
                "a-skipped",
                "./zeta/packages/skipped",
                Status::NoRemote,
                "no remote",
                false,
            ),
            (
                "z-skipped",
                "./alpha/packages/skipped",
                Status::NoRemote,
                "no remote",
                false,
            ),
            (
                "a-follow-up",
                "./zeta/packages/follow-up",
                Status::Synced,
                "up to date",
                true,
            ),
            (
                "z-follow-up",
                "./alpha/packages/follow-up",
                Status::Synced,
                "up to date",
                true,
            ),
        ] {
            pull.update(name, path, &status, message, has_uncommitted);
        }
        let push = SyncStatistics::new();

        let report = generate_sync_report(&pull, &push, Duration::ZERO, 8, false, 0, &[]);

        for suffix in [
            "apps/pulled",
            "packages/failed",
            "packages/skipped",
            "packages/follow-up",
        ] {
            let alpha = format!("path: ./alpha/{suffix}");
            let zeta = format!("path: ./zeta/{suffix}");
            let alpha_index = report
                .find(&alpha)
                .unwrap_or_else(|| panic!("missing {alpha} in report"));
            let zeta_index = report
                .find(&zeta)
                .unwrap_or_else(|| panic!("missing {zeta} in report"));
            assert!(
                alpha_index < zeta_index,
                "expected {alpha} before {zeta}\n{report}"
            );
        }
    }
}
