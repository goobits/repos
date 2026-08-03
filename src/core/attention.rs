//! Project-grouped attention reporting for fleet operations.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use crate::utils::compare_repository_locations;

use super::stats::{
    format_relative_repo_path, get_repo_changes, truncate_text, BOLD_BLUE, BOLD_PURPLE, DIM, RED,
    RESET, YELLOW,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AttentionKind {
    Failed,
    Skipped,
    FollowUp,
}

impl AttentionKind {
    fn label(self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::FollowUp => "follow-up",
        }
    }

    fn marker(self) -> &'static str {
        match self {
            Self::Failed => "!",
            Self::Skipped => "·",
            Self::FollowUp => "~",
        }
    }

    fn color(self) -> &'static str {
        match self {
            Self::Failed => RED,
            Self::Skipped => DIM,
            Self::FollowUp => YELLOW,
        }
    }
}

pub(super) struct ProjectAttention {
    project: String,
    repository: String,
    path: Option<String>,
    kind: AttentionKind,
    message: String,
    next: String,
    remote: Option<String>,
}

impl ProjectAttention {
    pub(super) fn new(
        kind: AttentionKind,
        repository: impl Into<String>,
        path: impl Into<String>,
        message: impl Into<String>,
        next: impl Into<String>,
        remote: Option<String>,
    ) -> Self {
        let repository = repository.into();
        let path = path.into();
        Self {
            project: project_name(&path, &repository),
            repository,
            path: Some(path),
            kind,
            message: message.into(),
            next: next.into(),
            remote,
        }
    }

    pub(super) fn unattributed(
        kind: AttentionKind,
        repository: impl Into<String>,
        message: impl Into<String>,
        next: impl Into<String>,
    ) -> Self {
        Self {
            project: "Fleet".to_string(),
            repository: repository.into(),
            path: None,
            kind,
            message: message.into(),
            next: next.into(),
            remote: None,
        }
    }
}

fn project_name(path: &str, repository: &str) -> String {
    let display_path = format_relative_repo_path(path);
    let display_path = Path::new(&display_path);
    if display_path.is_absolute() {
        return repository.to_string();
    }

    display_path
        .components()
        .find_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .unwrap_or_else(|| repository.to_string())
}

pub(super) fn append_project_attention_section(
    lines: &mut Vec<String>,
    items: Vec<ProjectAttention>,
    show_changes: bool,
) {
    if items.is_empty() {
        return;
    }

    let mut projects = BTreeMap::<String, Vec<ProjectAttention>>::new();
    for item in items {
        projects.entry(item.project.clone()).or_default().push(item);
    }

    if !lines.last().is_some_and(String::is_empty) {
        lines.push(String::new());
    }
    lines.push(format!("{BOLD_PURPLE}▌ Needs Attention by Project{RESET}"));

    for (project_index, (project, mut project_items)) in projects.into_iter().enumerate() {
        if project_index > 0 {
            lines.push(String::new());
        }
        project_items.sort_by(|left, right| {
            compare_repository_locations(
                left.path.as_deref().unwrap_or_default(),
                &left.repository,
                right.path.as_deref().unwrap_or_default(),
                &right.repository,
            )
        });

        let issue_label = if project_items.len() == 1 {
            "issue"
        } else {
            "issues"
        };
        lines.push(format!(
            "  {BOLD_BLUE}{project}{RESET} {DIM}({} {issue_label}){RESET}",
            project_items.len()
        ));
        lines.push(format!("  {DIM}────────────────────{RESET}"));

        for item in project_items {
            lines.push(format!(
                "    {}{}{RESET} {:<9} {:24} {}",
                item.kind.color(),
                item.kind.marker(),
                item.kind.label(),
                truncate_text(&item.repository, 24),
                item.message
            ));
            if let Some(path) = item.path.as_deref() {
                lines.push(format!(
                    "      {DIM}↳ path: {}{RESET}",
                    format_relative_repo_path(path)
                ));
            }
            if let Some(remote) = item.remote {
                lines.push(format!("      {DIM}↳ remote: {remote}{RESET}"));
            }
            lines.push(format!("      {DIM}↳ next: {}{RESET}", item.next));

            if show_changes && item.kind == AttentionKind::FollowUp {
                append_changes(lines, item.path.as_deref());
            }
        }
    }
}

fn append_changes(lines: &mut Vec<String>, path: Option<&str>) {
    let Some(path) = path else {
        return;
    };
    let Ok(changes) = get_repo_changes(path) else {
        return;
    };
    for change in changes {
        lines.push(format!("        {DIM}· {change}{RESET}"));
    }
}

#[cfg(test)]
mod tests {
    use super::{append_project_attention_section, AttentionKind, ProjectAttention};

    #[test]
    fn kinds_use_distinct_single_character_markers() {
        let markers = [
            AttentionKind::Failed.marker(),
            AttentionKind::Skipped.marker(),
            AttentionKind::FollowUp.marker(),
        ];

        assert_eq!(markers, ["!", "·", "~"]);
        assert!(markers.iter().all(|marker| marker.chars().count() == 1));
    }

    #[test]
    fn attention_is_grouped_by_project_then_sorted_by_repository_path() {
        let items = vec![
            ProjectAttention::new(
                AttentionKind::Skipped,
                "zeta-package",
                "./zeta/packages/shared",
                "detached HEAD",
                "checkout a branch",
                None,
            ),
            ProjectAttention::new(
                AttentionKind::FollowUp,
                "alpha-package",
                "./alpha/packages/shared",
                "uncommitted changes",
                "commit or stash the local changes",
                None,
            ),
            ProjectAttention::new(
                AttentionKind::Failed,
                "alpha",
                "./alpha",
                "authentication failed",
                "inspect failure",
                None,
            ),
        ];
        let mut lines = Vec::new();

        append_project_attention_section(&mut lines, items, false);
        let report = lines.join("\n");

        assert_eq!(report.matches("▌ Needs Attention by Project").count(), 1);
        let alpha_project = report.find("alpha\x1b[0m").expect("alpha project");
        let zeta_project = report.find("zeta\x1b[0m").expect("zeta project");
        let alpha_root = report.find("path: ./alpha\x1b[0m").expect("alpha root");
        let alpha_package = report
            .find("path: ./alpha/packages/shared")
            .expect("alpha package");
        assert!(alpha_project < zeta_project, "{report}");
        assert!(alpha_root < alpha_package, "{report}");
        assert!(report.contains("!\x1b[0m failed"));
        assert!(report.contains("·\x1b[0m skipped"));
        assert!(report.contains("~\x1b[0m follow-up"));
    }
}
