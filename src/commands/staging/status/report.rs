//! Deterministic fleet-status report rendering.

use super::*;

pub(super) fn generate_status_report(
    entries: &[FleetStatusEntry],
    filters: StatusFilters,
    duration: std::time::Duration,
) -> String {
    let mut entries = entries.iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        compare_repository_locations(&left.path, &left.repository, &right.path, &right.repository)
    });

    let healthy = entries
        .iter()
        .filter(|entry| entry.status.kind() == FleetStatusKind::Healthy)
        .count();
    let needs_work = entries
        .iter()
        .filter(|entry| entry.status.kind() == FleetStatusKind::NeedsWork)
        .count();
    let failed = entries
        .iter()
        .filter(|entry| entry.status.kind() == FleetStatusKind::Failed)
        .count();
    let shown = entries
        .iter()
        .filter(|entry| entry.status.matches_filters(filters))
        .count();

    let mut lines = vec![
        format!("{BOLD_BLUE}repos status{RESET}"),
        format!(
            "{GREEN}✓{RESET} Completed in {:.1}s",
            duration.as_secs_f64()
        ),
        String::new(),
        format!("{BOLD_PURPLE}▌ Summary{RESET}"),
        format!("  {GREEN}✓{RESET} {:<16}{healthy}", "Healthy"),
    ];
    if needs_work > 0 {
        lines.push(format!(
            "  {YELLOW}!{RESET} {:<16}{needs_work}",
            "Needs work"
        ));
    }
    if failed > 0 {
        lines.push(format!("  {RED}!{RESET} {:<16}{failed}", "Failed"));
    }
    if !filters.is_empty() {
        lines.push(format!("  {DIM}·{RESET} {:<16}{shown}", "Shown"));
    }
    lines.push(format!(
        "  {DIM}·{RESET} {:<16}{}",
        "Checked",
        entries.len()
    ));

    append_status_section(&mut lines, &entries, filters, FleetStatusKind::Failed);
    append_status_section(&mut lines, &entries, filters, FleetStatusKind::NeedsWork);
    append_status_section(&mut lines, &entries, filters, FleetStatusKind::Healthy);

    if shown == 0 {
        lines.push(String::new());
        lines.push(format!("{BOLD_PURPLE}▌ Repositories{RESET}"));
        lines.push(format!(
            "  {DIM}No repositories matched the requested status filters.{RESET}"
        ));
    }

    lines.join("\n")
}

fn append_status_section(
    lines: &mut Vec<String>,
    entries: &[&FleetStatusEntry],
    filters: StatusFilters,
    kind: FleetStatusKind,
) {
    let matching = entries
        .iter()
        .filter(|entry| entry.status.kind() == kind && entry.status.matches_filters(filters))
        .copied()
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return;
    }

    lines.push(String::new());
    lines.push(format!("{BOLD_PURPLE}▌ {}{RESET}", kind.heading()));
    let (color, marker) = kind.style();
    for entry in matching {
        lines.push(format!(
            "  {color}{marker}{RESET} {:24} {}",
            truncate_text(&entry.repository, 24),
            entry.status.message
        ));
        lines.push(format!(
            "    {DIM}↳ path: {}{RESET}",
            format_relative_repo_path(&entry.path.to_string_lossy())
        ));
        if let Some(remote) = entry
            .status
            .failure
            .as_ref()
            .and_then(|failure| failure.remote.as_ref())
        {
            lines.push(format!("    {DIM}↳ remote: {}{RESET}", remote.display()));
        }
        if let Some(next) = entry.status.next_action(&entry.path) {
            lines.push(format!("    {DIM}↳ next: {next}{RESET}"));
        }
    }
}
