//! Doctor result models and deterministic terminal rendering.

use crate::utils::compare_repository_locations;

const RESET: &str = "\x1b[0m";
const BOLD_BLUE: &str = "\x1b[1;38;5;75m";
const BOLD_PURPLE: &str = "\x1b[1;38;5;141m";
const GREEN: &str = "\x1b[1;38;5;114m";
const YELLOW: &str = "\x1b[1;38;5;221m";
const RED: &str = "\x1b[1;38;5;203m";
const DIM: &str = "\x1b[2m";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DoctorFinding {
    pub(super) message: String,
    pub(super) next: String,
}

impl DoctorFinding {
    pub(super) fn new(message: impl Into<String>, next: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            next: next.into(),
        }
    }
}

#[derive(Debug)]
pub(super) struct RepositoryDiagnosis {
    pub(super) repository: String,
    pub(super) path: String,
    pub(super) blockers: Vec<DoctorFinding>,
    pub(super) advisories: Vec<DoctorFinding>,
}

impl RepositoryDiagnosis {
    pub(super) fn new(repository: &str, path: &std::path::Path) -> Self {
        Self {
            repository: repository.to_string(),
            path: path.to_string_lossy().into_owned(),
            blockers: Vec::new(),
            advisories: Vec::new(),
        }
    }

    pub(super) fn finish(mut self) -> Self {
        self.blockers.sort_by(|left, right| {
            left.message
                .cmp(&right.message)
                .then_with(|| left.next.cmp(&right.next))
        });
        self.advisories.sort_by(|left, right| {
            left.message
                .cmp(&right.message)
                .then_with(|| left.next.cmp(&right.next))
        });
        self
    }

    pub(super) fn progress_label(&self) -> &'static str {
        if !self.blockers.is_empty() {
            "blocked"
        } else if !self.advisories.is_empty() {
            "warning"
        } else {
            "healthy"
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct DoctorReport {
    pub(super) repositories: Vec<RepositoryDiagnosis>,
    pub(super) nested_drift_count: usize,
    pub(super) nested_drift_lines: Vec<String>,
    pub(super) global_advisories: Vec<DoctorFinding>,
}

impl DoctorReport {
    pub(super) fn blocker_repos(&self) -> usize {
        self.repositories
            .iter()
            .filter(|diagnosis| !diagnosis.blockers.is_empty())
            .count()
    }

    fn warning_repos(&self) -> usize {
        self.repositories
            .iter()
            .filter(|diagnosis| diagnosis.blockers.is_empty() && !diagnosis.advisories.is_empty())
            .count()
    }

    fn healthy_repos(&self) -> usize {
        self.repositories
            .iter()
            .filter(|diagnosis| diagnosis.blockers.is_empty() && diagnosis.advisories.is_empty())
            .count()
    }

    fn warning_count(&self) -> usize {
        self.warning_repos() + self.global_advisories.len()
    }

    pub(super) fn render(&self, duration: std::time::Duration) -> String {
        let mut lines = vec![
            format!("{BOLD_BLUE}repos doctor{RESET}"),
            format!(
                "{GREEN}✓{RESET} Completed in {:.1}s",
                duration.as_secs_f64()
            ),
            String::new(),
            format!("{BOLD_PURPLE}▌ Summary{RESET}"),
            format!(
                "  {GREEN}✓{RESET} {:<16}{}",
                "Healthy",
                self.healthy_repos()
            ),
        ];
        if self.warning_count() > 0 {
            lines.push(format!(
                "  {YELLOW}!{RESET} {:<16}{}",
                "Warnings",
                self.warning_count()
            ));
        }
        if self.blocker_repos() > 0 {
            lines.push(format!(
                "  {RED}!{RESET} {:<16}{}",
                "Blockers",
                self.blocker_repos()
            ));
        }
        if self.nested_drift_count > 0 {
            lines.push(format!(
                "  {RED}!{RESET} {:<16}{}",
                "Nested drift", self.nested_drift_count
            ));
        }
        lines.push(format!(
            "  {DIM}·{RESET} {:<16}{}",
            "Checked",
            self.repositories.len()
        ));

        append_diagnosis_section(
            &mut lines,
            "Blockers",
            RED,
            &self.repositories,
            |diagnosis| &diagnosis.blockers,
        );
        if !self.nested_drift_lines.is_empty() {
            lines.push(String::new());
            lines.extend(self.nested_drift_lines.iter().cloned());
        }
        append_diagnosis_section(
            &mut lines,
            "Advisories",
            YELLOW,
            &self.repositories,
            |diagnosis| &diagnosis.advisories,
        );
        if !self.global_advisories.is_empty() {
            lines.push(String::new());
            if !self
                .repositories
                .iter()
                .any(|diagnosis| !diagnosis.advisories.is_empty())
            {
                lines.push(format!("{BOLD_PURPLE}▌ Advisories{RESET}"));
            }
            for advisory in &self.global_advisories {
                lines.push(format!("  {YELLOW}!{RESET} {}", advisory.message));
                lines.push(format!("    {DIM}↳ next: {}{RESET}", advisory.next));
            }
        }

        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }
}

fn append_diagnosis_section<F>(
    lines: &mut Vec<String>,
    heading: &str,
    color: &str,
    diagnoses: &[RepositoryDiagnosis],
    select: F,
) where
    F: Fn(&RepositoryDiagnosis) -> &[DoctorFinding],
{
    let mut matching = diagnoses
        .iter()
        .filter(|diagnosis| !select(diagnosis).is_empty())
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return;
    }
    matching.sort_by(|left, right| {
        compare_repository_locations(&left.path, &left.repository, &right.path, &right.repository)
    });

    lines.push(String::new());
    lines.push(format!("{BOLD_PURPLE}▌ {heading}{RESET}"));
    for diagnosis in matching {
        lines.push(format!("  {color}!{RESET} {}", diagnosis.repository));
        lines.push(format!("    {DIM}↳ path: {}{RESET}", diagnosis.path));
        for finding in select(diagnosis) {
            lines.push(format!("    {color}·{RESET} {}", finding.message));
            lines.push(format!("      {DIM}↳ next: {}{RESET}", finding.next));
        }
    }
}
