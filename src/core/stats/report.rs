//! Transfer-specific report composition.

use super::*;

pub(super) fn append_transfer_summary(
    lines: &mut Vec<String>,
    transfer: Transfer,
    totals: TransferTotals,
) {
    let transfer_label = transfer.label();
    let repository_label = pluralize(totals.transferred_repos, "repo", "repos");
    let unit_label = transfer.unit(totals.transferred_commits);

    lines.push(format!("{BOLD_PURPLE}▌ Summary{RESET}"));
    lines.push(format!(
        "  {GREEN}✓{RESET} {transfer_label:<13}{} {repository_label} / {} {unit_label}",
        totals.transferred_repos, totals.transferred_commits
    ));
    lines.push(format!(
        "  {GREEN}✓{RESET} {:<13}{}",
        "Up to date", totals.up_to_date
    ));
    if totals.errors > 0 {
        lines.push(format!("  {RED}!{RESET} {:<13}{}", "Failed", totals.errors));
    }
    if totals.skipped > 0 {
        lines.push(format!(
            "  {DIM}·{RESET} {:<13}{}",
            "Skipped", totals.skipped
        ));
    }
    if totals.follow_up > 0 {
        lines.push(format!(
            "  {YELLOW}~{RESET} {:<13}{}",
            "Follow-up", totals.follow_up
        ));
    }
    lines.push(format!(
        "  {DIM}·{RESET} {:<13}{}",
        "Checked", totals.checked
    ));
}

pub(super) fn clone_vec<T: Clone>(values: &Mutex<Vec<T>>, label: &str) -> Vec<T> {
    match values.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => {
            eprintln!("Warning: Failed to acquire lock for {label}");
            Vec::new()
        }
    }
}

pub(super) fn clone_failure_map(
    failures: &Mutex<HashMap<(String, String), GitFailure>>,
) -> HashMap<(String, String), GitFailure> {
    match failures.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => {
            eprintln!("Warning: Failed to acquire lock for git_failures");
            HashMap::new()
        }
    }
}

pub(super) fn transfer_failure_attention(
    transfer: Transfer,
    errors: u64,
    failed_repos: &[(String, String, String)],
    git_failures: &HashMap<(String, String), GitFailure>,
) -> Vec<ProjectAttention> {
    if errors == 0 {
        return Vec::new();
    }

    if failed_repos.is_empty() {
        return vec![ProjectAttention::unattributed(
            AttentionKind::Failed,
            "repositories",
            format!("{errors} repositories failed without details"),
            "run `repos doctor`",
        )];
    }

    failed_repos
        .iter()
        .map(|(repo_name, repo_path, error)| {
            let failure = git_failures.get(&(repo_name.clone(), repo_path.clone()));
            let reason = failure
                .map(GitFailure::reason)
                .unwrap_or_else(|| compact_git_error(error));
            let display_path = format_relative_repo_path(repo_path);
            let next = failure.map_or_else(
                || next_for_git_error(error, transfer),
                |failure| failure.next_action(&display_path),
            );
            let remote = failure
                .and_then(|failure| failure.remote.as_ref())
                .map(|remote| remote.display());
            ProjectAttention::new(
                AttentionKind::Failed,
                repo_name.clone(),
                repo_path.clone(),
                reason,
                next,
                remote,
            )
        })
        .collect()
}

pub(super) fn is_transfer_success(status: Status) -> bool {
    matches!(
        status,
        Status::Synced | Status::Fetched | Status::Pushed | Status::Pulled
    )
}

pub(super) fn is_transfer_skip(status: Status) -> bool {
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

pub(super) fn transfer_skip_attention(
    transfer: Transfer,
    outcomes: &[&RepositoryOutcome],
) -> Vec<ProjectAttention> {
    outcomes
        .iter()
        .map(|outcome| {
            let mut reason = clean_error_message(&outcome.message);
            if outcome.has_uncommitted && !reason.contains("uncommitted") {
                reason.push_str(" + uncommitted changes");
            }
            ProjectAttention::new(
                AttentionKind::Skipped,
                outcome.repository.clone(),
                outcome.path.clone(),
                reason,
                transfer_skip_next(transfer, outcome),
                None,
            )
        })
        .collect()
}

fn transfer_skip_next(transfer: Transfer, outcome: &RepositoryOutcome) -> &'static str {
    match outcome.status {
        Status::NoRemote => "add remote or skip",
        Status::NoUpstream => transfer.no_upstream_action(),
        Status::Dirty => "commit or stash local changes, then retry",
        Status::Skip if outcome.message.contains("detached HEAD") => "checkout a branch",
        Status::NoChanges => "no action",
        _ => "run `repos status --skipped`",
    }
}

fn compact_git_error(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("diverged") {
        return error
            .replace(" (run repos sync or resolve manually)", "")
            .replace(", ", " / ");
    }
    clean_error_message(error)
}

fn next_for_git_error(error: &str, transfer: Transfer) -> String {
    let lower = error.to_lowercase();
    if lower.contains("diverged") {
        "repos sync or resolve manually".to_string()
    } else if lower.contains("repository moved") && lower.contains("email privacy") {
        match transfer {
            Transfer::Fetch => "update remote, then fetch".to_string(),
            Transfer::Push => "update remote + fix git email".to_string(),
            Transfer::Pull => "update remote, then pull".to_string(),
        }
    } else if lower.contains("email privacy") {
        match transfer {
            Transfer::Fetch | Transfer::Pull => "inspect failure".to_string(),
            Transfer::Push => "fix git email, then push".to_string(),
        }
    } else if lower.contains("repository moved") {
        format!("update remote, then {}", transfer.command())
    } else {
        "inspect failure".to_string()
    }
}
