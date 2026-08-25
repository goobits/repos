//! Outcome models and terminal reports for nested checkout mutations.

use super::super::SubrepoInstance;
use crate::core::{format_relative_repo_path, truncate_text};
use crate::utils::compare_repository_locations;
use anyhow::Result;

const RESET: &str = "\x1b[0m";
const BOLD_BLUE: &str = "\x1b[1;38;5;75m";
const BOLD_PURPLE: &str = "\x1b[1;38;5;141m";
const GREEN: &str = "\x1b[1;38;5;114m";
const YELLOW: &str = "\x1b[1;38;5;221m";
const RED: &str = "\x1b[1;38;5;203m";
const DIM: &str = "\x1b[2m";

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum NestedOperation {
    Sync,
    Update,
}

impl NestedOperation {
    fn command(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Update => "update",
        }
    }

    fn changed_label(self) -> &'static str {
        match self {
            Self::Sync => "Synced",
            Self::Update => "Updated",
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum NestedOutcomeKind {
    Changed,
    Unchanged,
    Skipped,
    Failed,
}

pub(super) struct NestedOutcome {
    pub(super) repository: String,
    pub(super) path: String,
    pub(super) kind: NestedOutcomeKind,
    pub(super) message: String,
    pub(super) next: Option<String>,
}

impl NestedOutcome {
    pub(super) fn new(
        instance: &SubrepoInstance,
        kind: NestedOutcomeKind,
        message: impl Into<String>,
        next: Option<String>,
    ) -> Self {
        Self {
            repository: instance.parent_repo.clone(),
            path: instance.subrepo_path.to_string_lossy().into_owned(),
            kind,
            message: message.into(),
            next,
        }
    }
}

pub(super) fn generate_operation_report(
    operation: NestedOperation,
    outcomes: &[NestedOutcome],
) -> String {
    let mut outcomes = outcomes.iter().collect::<Vec<_>>();
    outcomes.sort_by(|left, right| {
        compare_repository_locations(&left.path, &left.repository, &right.path, &right.repository)
    });
    let count = |kind| {
        outcomes
            .iter()
            .filter(|outcome| outcome.kind == kind)
            .count()
    };
    let changed = count(NestedOutcomeKind::Changed);
    let unchanged = count(NestedOutcomeKind::Unchanged);
    let skipped = count(NestedOutcomeKind::Skipped);
    let failed = count(NestedOutcomeKind::Failed);

    let mut lines = vec![
        format!("{BOLD_BLUE}repos nested {}{RESET}", operation.command()),
        String::new(),
        format!("{BOLD_PURPLE}▌ Summary{RESET}"),
        format!(
            "  {GREEN}✓{RESET} {:<16}{changed}",
            operation.changed_label()
        ),
    ];
    if unchanged > 0 {
        lines.push(format!("  {GREEN}✓{RESET} {:<16}{unchanged}", "Up to date"));
    }
    if skipped > 0 {
        lines.push(format!("  {YELLOW}!{RESET} {:<16}{skipped}", "Skipped"));
    }
    if failed > 0 {
        lines.push(format!("  {RED}!{RESET} {:<16}{failed}", "Failed"));
    }
    lines.push(format!(
        "  {DIM}·{RESET} {:<16}{}",
        "Checked",
        outcomes.len()
    ));

    append_operation_section(
        &mut lines,
        &outcomes,
        NestedOutcomeKind::Changed,
        operation.changed_label(),
        GREEN,
        "✓",
    );
    append_operation_section(
        &mut lines,
        &outcomes,
        NestedOutcomeKind::Unchanged,
        "Up to Date",
        GREEN,
        "✓",
    );
    append_operation_section(
        &mut lines,
        &outcomes,
        NestedOutcomeKind::Skipped,
        "Skipped",
        YELLOW,
        "!",
    );
    append_operation_section(
        &mut lines,
        &outcomes,
        NestedOutcomeKind::Failed,
        "Failed",
        RED,
        "!",
    );

    lines.join("\n")
}

fn append_operation_section(
    lines: &mut Vec<String>,
    outcomes: &[&NestedOutcome],
    kind: NestedOutcomeKind,
    heading: &str,
    color: &str,
    marker: &str,
) {
    let matching = outcomes
        .iter()
        .filter(|outcome| outcome.kind == kind)
        .copied()
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return;
    }

    lines.push(String::new());
    lines.push(format!("{BOLD_PURPLE}▌ {heading}{RESET}"));
    for outcome in matching {
        lines.push(format!(
            "  {color}{marker}{RESET} {:24} {}",
            truncate_text(&outcome.repository, 24),
            outcome.message
        ));
        lines.push(format!(
            "    {DIM}↳ path: {}{RESET}",
            format_relative_repo_path(&outcome.path)
        ));
        if let Some(next) = &outcome.next {
            lines.push(format!("    {DIM}↳ next: {next}{RESET}"));
        }
    }
}

pub(super) fn finish_operation(
    operation: NestedOperation,
    outcomes: &[NestedOutcome],
) -> Result<()> {
    println!("\n{}\n", generate_operation_report(operation, outcomes));

    let error_count = outcomes
        .iter()
        .filter(|outcome| outcome.kind == NestedOutcomeKind::Failed)
        .count();
    if error_count > 0 {
        anyhow::bail!(
            "{error_count} repositories failed to {}",
            operation.command()
        );
    }
    Ok(())
}

pub(super) fn record_aborted_candidates<'a>(
    candidates: impl IntoIterator<Item = &'a SubrepoInstance>,
    outcomes: &mut Vec<NestedOutcome>,
) {
    for instance in candidates {
        outcomes.push(NestedOutcome::new(
            instance,
            NestedOutcomeKind::Skipped,
            "batch preflight failed; no changes applied",
            Some("resolve the reported failure, then retry".to_string()),
        ));
    }
}
