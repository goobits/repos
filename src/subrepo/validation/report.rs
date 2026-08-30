use super::{NestedCheckoutKind, SubrepoInstance, ValidationReport};

const RESET: &str = "\x1b[0m";
const BOLD_BLUE: &str = "\x1b[1;38;5;75m";
const BOLD_PURPLE: &str = "\x1b[1;38;5;141m";
const GREEN: &str = "\x1b[1;38;5;114m";
const YELLOW: &str = "\x1b[1;38;5;221m";
const DIM: &str = "\x1b[2m";

/// Display the validation report.
pub fn display_report(report: &ValidationReport) {
    println!("{}", generate_validation_report(report));
}

pub(super) fn generate_validation_report(report: &ValidationReport) -> String {
    let shared = report.shared_subrepos_count();
    let unique = report.unique_remotes().saturating_sub(shared);
    let mut lines = vec![
        format!("{BOLD_BLUE}repos nested validate{RESET}"),
        String::new(),
        format!("{BOLD_PURPLE}▌ Summary{RESET}"),
        format!("  {GREEN}✓{RESET} {:<18}{shared}", "Shared groups"),
        format!("  {DIM}·{RESET} {:<18}{unique}", "Unique groups"),
    ];
    if !report.no_remote.is_empty() {
        lines.push(format!(
            "  {YELLOW}!{RESET} {:<18}{}",
            "Missing remote",
            report.no_remote.len()
        ));
    }
    if !report.uninitialized_submodules.is_empty() {
        lines.push(format!(
            "  {YELLOW}!{RESET} {:<18}{}",
            "Uninitialized",
            report.uninitialized_submodules.len()
        ));
    }
    lines.push(format!(
        "  {DIM}·{RESET} {:<18}{}",
        "Nested copies", report.total_nested
    ));
    lines.push(format!(
        "  {DIM}·{RESET} {:<18}{}",
        "Independent",
        report.checkout_count(NestedCheckoutKind::Independent)
    ));
    lines.push(format!(
        "  {DIM}·{RESET} {:<18}{}",
        "Submodules",
        report.checkout_count(NestedCheckoutKind::Submodule)
    ));
    lines.push(format!(
        "  {DIM}·{RESET} {:<18}{}",
        "Linked worktrees",
        report.checkout_count(NestedCheckoutKind::LinkedWorktree)
    ));

    if report.total_nested == 0 && report.uninitialized_submodules.is_empty() {
        lines.push(String::new());
        lines.push(format!("{BOLD_PURPLE}▌ Result{RESET}"));
        lines.push(format!("  {DIM}No nested repositories found.{RESET}"));
        return lines.join("\n");
    }

    let mut groups = report.by_remote.iter().collect::<Vec<_>>();
    groups.sort_by_key(|(remote, _)| *remote);
    if !groups.is_empty() {
        lines.push(String::new());
        lines.push(format!("{BOLD_PURPLE}▌ Nested Repositories{RESET}"));
        for (remote, instances) in groups {
            let mut instances = instances.iter().collect::<Vec<_>>();
            instances.sort_by(|left, right| {
                left.parent_repo
                    .cmp(&right.parent_repo)
                    .then_with(|| left.relative_path.cmp(&right.relative_path))
            });
            let name = &instances[0].subrepo_name;
            let copies = instances.len();
            let group_kind = if copies > 1 { "shared" } else { "unique" };
            lines.push(format!(
                "  {GREEN}✓{RESET} {name} ({copies} copies, {group_kind})"
            ));
            lines.push(format!("    {DIM}↳ remote: {remote}{RESET}"));
            for instance in instances {
                let state = if instance.has_uncommitted {
                    "uncommitted changes"
                } else {
                    "clean"
                };
                lines.push(format!(
                    "    · {} @ {} ({state}, {})",
                    nested_location(instance),
                    instance.short_hash,
                    instance.checkout_kind.label()
                ));
            }
        }
    }

    if !report.no_remote.is_empty() {
        let mut missing = report.no_remote.iter().collect::<Vec<_>>();
        missing.sort_by(|left, right| {
            left.parent_repo
                .cmp(&right.parent_repo)
                .then_with(|| left.relative_path.cmp(&right.relative_path))
        });
        lines.push(String::new());
        lines.push(format!("{BOLD_PURPLE}▌ Missing Remote{RESET}"));
        for instance in missing {
            lines.push(format!(
                "  {YELLOW}!{RESET} {} @ {} ({})",
                nested_location(instance),
                instance.short_hash,
                instance.checkout_kind.label()
            ));
            lines.push(format!(
                "    {DIM}↳ next: add an origin remote or exclude this nested repository{RESET}"
            ));
        }
    }

    if !report.uninitialized_submodules.is_empty() {
        lines.push(String::new());
        lines.push(format!("{BOLD_PURPLE}▌ Uninitialized Submodules{RESET}"));
        for submodule in &report.uninitialized_submodules {
            lines.push(format!(
                "  {YELLOW}!{RESET} {}/{} @ {}",
                submodule.parent_repo,
                submodule.relative_path,
                submodule.target_commit.chars().take(7).collect::<String>()
            ));
            lines.push(format!(
                "    {DIM}↳ next: git -C {} submodule update --init -- {}{RESET}",
                submodule.parent_path.display(),
                submodule.relative_path
            ));
        }
    }

    lines.push(String::new());
    lines.push(format!("{BOLD_PURPLE}▌ Result{RESET}"));
    if shared > 0 {
        lines.push(format!(
            "  {GREEN}✓{RESET} Drift tracking applies to {shared} shared nested group{}.",
            if shared == 1 { "" } else { "s" }
        ));
        lines.push(format!(
            "    {DIM}↳ next: run `repos nested status` to inspect commit drift{RESET}"
        ));
    } else {
        lines.push(format!(
            "  {DIM}No nested remote is shared across parent repositories, so cross-copy drift cannot occur.{RESET}"
        ));
    }

    lines.join("\n")
}

fn nested_location(instance: &SubrepoInstance) -> String {
    match instance.relative_path.as_str() {
        "" | "." => instance.parent_repo.clone(),
        relative => format!("{}/{relative}", instance.parent_repo),
    }
}
