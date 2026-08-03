//! Shared reporting for fleet-wide repository mutations.

use std::time::Duration;

use crate::git::Status;

use super::stats::{
    clean_error_message, SyncStatistics, BOLD_BLUE, BOLD_PURPLE, DIM, GREEN, RED, RESET, YELLOW,
};

#[derive(Clone, Debug)]
pub(crate) struct RepositoryOutcome {
    pub repository: String,
    pub path: String,
    pub status: Status,
    pub message: String,
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
            left.repository
                .cmp(&right.repository)
                .then_with(|| left.path.cmp(&right.path))
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

    fn batch_outcomes(&self) -> Vec<RepositoryOutcome> {
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
            truncate(&outcome.repository, 24)
        ));
        lines.push(format!("    ↳ path: {}", outcome.path));
        if section.actionable {
            lines.push(format!("    ↳ next: {}", operation.next_action(outcome)));
        }
    }
}

fn truncate(value: &str, max_chars: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return value.to_string();
    }
    chars[..max_chars.saturating_sub(1)]
        .iter()
        .collect::<String>()
        + "…"
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
}
