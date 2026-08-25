//! Exact local and upstream state inspection for fleet status.

use super::*;

pub(super) async fn get_fleet_status(
    repo_path: &std::path::Path,
    show_details: bool,
) -> FleetStatus {
    let initial_state =
        match crate::git::worktree::inspect_refreshed_repository_state(repo_path).await {
            Ok(state) => state,
            Err(error) => {
                return FleetStatus {
                    status: Status::StagingError,
                    worktree_status: Status::StagingError,
                    message: format!("status failed: {}", clean_error_message(&error.to_string())),
                    upstream: UpstreamSummary::Unknown,
                    failure: None,
                };
            }
        };

    let (working_status, mut parts, details) = match get_staging_status(repo_path).await {
        Ok((stdout, _)) => summarize_worktree(&stdout, show_details),
        Err(error) => {
            return FleetStatus {
                status: Status::StagingError,
                worktree_status: Status::StagingError,
                message: format!("status failed: {}", clean_error_message(&error.to_string())),
                upstream: UpstreamSummary::Unknown,
                failure: None,
            };
        }
    };

    let branch = match initial_state.head() {
        crate::git::worktree::HeadState::Branch(branch) => branch.clone(),
        crate::git::worktree::HeadState::Detached => "HEAD".to_string(),
        crate::git::worktree::HeadState::Unborn => "unborn".to_string(),
        crate::git::worktree::HeadState::Unknown => "unknown".to_string(),
    };
    parts.insert(0, format!("branch {branch}"));

    let refresh =
        crate::git::operations::fetch_and_analyze_for_pull_with_state(repo_path, initial_state)
            .await;
    if refresh.status == Status::Error {
        let failure = refresh.failure;
        let reason = failure
            .as_ref()
            .map_or_else(|| clean_error_message(&refresh.message), GitFailure::reason);
        parts.push(reason);
        let mut message = parts.join(" | ");
        if !details.is_empty() {
            message.push('\n');
            message.push_str(&details.join("\n"));
        }
        return FleetStatus {
            status: Status::Error,
            worktree_status: working_status,
            message,
            upstream: UpstreamSummary::Unknown,
            failure,
        };
    }

    let upstream = match refresh.status {
        Status::NoRemote => UpstreamSummary::NoRemote,
        Status::NoUpstream => UpstreamSummary::NoUpstream,
        Status::Skip => UpstreamSummary::Unknown,
        _ => UpstreamSummary::from_counts(
            refresh.upstream_name.as_deref().unwrap_or("upstream"),
            refresh.ahead_count,
            refresh.behind_count,
        ),
    };
    if let Some(summary) = upstream.message() {
        parts.push(summary.to_string());
    }

    let mut message = parts.join(" | ");
    if !details.is_empty() {
        message.push('\n');
        message.push_str(&details.join("\n"));
    }

    FleetStatus {
        status: working_status,
        worktree_status: working_status,
        message,
        upstream,
        failure: None,
    }
}

pub(super) fn summarize_worktree(
    stdout: &str,
    show_details: bool,
) -> (Status, Vec<String>, Vec<String>) {
    if stdout.trim().is_empty() {
        return (Status::Synced, vec!["clean".to_string()], Vec::new());
    }

    let lines: Vec<&str> = stdout.lines().filter(|line| !line.is_empty()).collect();
    let staged_count = lines
        .iter()
        .filter(|line| {
            let chars: Vec<char> = line.chars().collect();
            chars.len() >= 2 && chars[0] != ' ' && chars[0] != '?'
        })
        .count();
    let unstaged_count = lines
        .iter()
        .filter(|line| {
            let chars: Vec<char> = line.chars().collect();
            chars.len() >= 2 && chars[1] != ' ' && !line.starts_with("??")
        })
        .count();
    let untracked_count = lines.iter().filter(|line| line.starts_with("??")).count();

    let mut parts = Vec::new();
    if staged_count > 0 {
        parts.push(format!("{staged_count} staged"));
    }
    if unstaged_count > 0 {
        parts.push(format!("{unstaged_count} unstaged"));
    }
    if untracked_count > 0 {
        parts.push(format!("{untracked_count} untracked"));
    }

    if parts.is_empty() {
        (Status::Synced, vec!["clean".to_string()], Vec::new())
    } else {
        let details = if show_details {
            format_status_details(&lines)
        } else {
            Vec::new()
        };
        (Status::Dirty, parts, details)
    }
}

pub(super) fn format_status_details(lines: &[&str]) -> Vec<String> {
    const MAX_FILES: usize = 20;

    lines
        .iter()
        .take(MAX_FILES)
        .map(|line| {
            let status = line.get(..2).unwrap_or(line);
            let path = line.get(3..).unwrap_or("").trim();
            format!("    {} {}", status_detail_label(status), path)
        })
        .chain(
            (lines.len() > MAX_FILES)
                .then(|| format!("    · ... and {} more", lines.len() - MAX_FILES)),
        )
        .collect()
}

fn status_detail_label(status: &str) -> &'static str {
    if status == "??" {
        "· untracked"
    } else if status.chars().next().is_some_and(|state| state != ' ') {
        "✓ staged  "
    } else if status.chars().nth(1).is_some_and(|state| state != ' ') {
        "! unstaged"
    } else {
        "· changed "
    }
}
