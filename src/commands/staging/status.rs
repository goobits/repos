//! Fleet status inspection and report rendering.

mod inspect;
mod report;

use super::*;
use inspect::get_fleet_status;
use report::generate_status_report;

#[cfg(test)]
use inspect::{format_status_details, summarize_worktree};

#[derive(Clone, Copy, Debug, Default)]
pub struct StatusFilters {
    pub needs_work: bool,
    pub dirty: bool,
    pub no_remote: bool,
    pub no_upstream: bool,
    pub failed: bool,
    pub skipped: bool,
}

impl StatusFilters {
    fn is_empty(self) -> bool {
        !self.needs_work
            && !self.dirty
            && !self.no_remote
            && !self.no_upstream
            && !self.failed
            && !self.skipped
    }
}

struct FleetStatus {
    status: Status,
    worktree_status: Status,
    message: String,
    upstream: UpstreamSummary,
    failure: Option<GitFailure>,
}

struct FleetStatusEntry {
    repository: String,
    path: PathBuf,
    status: FleetStatus,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum FleetStatusKind {
    Healthy,
    NeedsWork,
    Failed,
}

impl FleetStatusKind {
    fn heading(self) -> &'static str {
        match self {
            Self::Healthy => "Healthy",
            Self::NeedsWork => "Needs Work",
            Self::Failed => "Failed",
        }
    }

    fn style(self) -> (&'static str, &'static str) {
        match self {
            Self::Healthy => (GREEN, "✓"),
            Self::NeedsWork => (YELLOW, "!"),
            Self::Failed => (RED, "!"),
        }
    }
}

impl FleetStatus {
    fn matches_filters(&self, filters: StatusFilters) -> bool {
        if filters.is_empty() {
            return true;
        }

        (filters.needs_work && self.needs_work())
            || (filters.dirty && self.dirty())
            || (filters.no_remote && matches!(self.upstream, UpstreamSummary::NoRemote))
            || (filters.no_upstream && matches!(self.upstream, UpstreamSummary::NoUpstream))
            || (filters.failed && self.failed())
            || (filters.skipped && self.skipped_for_push())
    }

    fn needs_work(&self) -> bool {
        self.dirty()
            || matches!(
                self.upstream,
                UpstreamSummary::NoRemote | UpstreamSummary::NoUpstream
            )
            || self.upstream.needs_sync()
            || self.failed()
    }

    fn failed(&self) -> bool {
        matches!(
            self.status,
            Status::Error | Status::StagingError | Status::CommitError | Status::PullError
        )
    }

    fn skipped_for_push(&self) -> bool {
        !self.failed()
            && !self.upstream.is_diverged()
            && (matches!(
                self.upstream,
                UpstreamSummary::NoRemote | UpstreamSummary::NoUpstream
            ) || self.upstream.ahead() == Some(0))
    }

    fn dirty(&self) -> bool {
        self.worktree_status == Status::Dirty
    }

    fn kind(&self) -> FleetStatusKind {
        if self.failed() {
            FleetStatusKind::Failed
        } else if self.needs_work() {
            FleetStatusKind::NeedsWork
        } else {
            FleetStatusKind::Healthy
        }
    }

    fn next_action(&self, repo_path: &Path) -> Option<String> {
        if let Some(failure) = &self.failure {
            Some(failure.next_action(&format_relative_repo_path(&repo_path.to_string_lossy())))
        } else if self.failed() {
            Some("inspect the reported status failure".to_string())
        } else if self.dirty() {
            Some("commit or stash the local changes".to_string())
        } else if matches!(self.upstream, UpstreamSummary::NoRemote) {
            Some("add a remote or exclude this repository".to_string())
        } else if matches!(self.upstream, UpstreamSummary::NoUpstream) {
            Some("set an upstream branch or run `repos push --auto-upstream`".to_string())
        } else if self.upstream.is_diverged() {
            Some("run `repos sync` or resolve the divergence manually".to_string())
        } else if self.upstream.behind().is_some_and(|count| count > 0) {
            Some("run `repos pull`".to_string())
        } else if self.upstream.ahead().is_some_and(|count| count > 0) {
            Some("run `repos push`".to_string())
        } else {
            None
        }
    }
}

enum UpstreamSummary {
    Remote {
        message: String,
        ahead: u32,
        behind: u32,
    },
    NoRemote,
    NoUpstream,
    Unknown,
}

impl UpstreamSummary {
    fn from_counts(upstream: &str, ahead: u32, behind: u32) -> Self {
        let message = if ahead > 0 && behind > 0 {
            format!("diverged ({ahead} ahead, {behind} behind)")
        } else if ahead > 0 {
            format!("ahead {ahead}")
        } else if behind > 0 {
            format!("behind {behind}")
        } else {
            format!("synced with {upstream}")
        };
        Self::Remote {
            message,
            ahead,
            behind,
        }
    }

    fn message(&self) -> Option<&str> {
        match self {
            Self::Remote { message, .. } => Some(message),
            Self::NoRemote => Some("no remote"),
            Self::NoUpstream => Some("no upstream"),
            Self::Unknown => None,
        }
    }

    fn is_diverged(&self) -> bool {
        matches!(self, Self::Remote { ahead, behind, .. } if *ahead > 0 && *behind > 0)
    }

    fn needs_sync(&self) -> bool {
        matches!(self, Self::Remote { ahead, behind, .. } if *ahead > 0 || *behind > 0)
    }

    fn ahead(&self) -> Option<u32> {
        match self {
            Self::Remote { ahead, .. } => Some(*ahead),
            _ => None,
        }
    }

    fn behind(&self) -> Option<u32> {
        match self {
            Self::Remote { behind, .. } => Some(*behind),
            _ => None,
        }
    }
}

/// Processes all repositories concurrently for status checking.
pub(super) async fn process_status_repositories(
    context: crate::core::ProcessingContext,
    filters: StatusFilters,
) -> usize {
    use crate::core::{acquire_semaphore_permit, create_progress_bar};
    use futures::stream::{FuturesUnordered, StreamExt};

    let mut futures = FuturesUnordered::new();
    let show_details = context.repositories.len() == 1;
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in context.repositories.iter() {
        let progress_bar =
            create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
        progress_bar.set_message(STATUS_MESSAGE);
        repo_progress_bars.push(progress_bar);
    }

    let _separator = crate::core::create_separator_progress_bar(&context.multi_progress);
    let max_name_length = context.max_name_length;
    let start_time = context.start_time;
    for ((repo_name, repo_path), progress_bar) in
        context.repositories.iter().zip(repo_progress_bars)
    {
        let semaphore = std::sync::Arc::clone(&context.semaphore);
        let repository = repo_name.clone();
        let path = repo_path.clone();

        futures.push(async move {
            let _permit = acquire_semaphore_permit(&semaphore).await;
            let status = get_fleet_status(&path, show_details).await;

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.status.symbol(),
                repository,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<12}   {}", status.status.text(), status.message));
            progress_bar.finish_and_clear();

            FleetStatusEntry {
                repository,
                path,
                status,
            }
        });
    }

    let mut entries = Vec::with_capacity(context.total_repos);
    while let Some(entry) = futures.next().await {
        entries.push(entry);
    }

    let failed = entries.iter().filter(|entry| entry.status.failed()).count();
    print_final_report(&generate_status_report(
        &entries,
        filters,
        start_time.elapsed(),
    ));
    failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn filters_status_repositories_by_name() {
        let repos = vec![
            ("frontdesk".to_string(), PathBuf::from("./frontdesk")),
            ("tunajack.com".to_string(), PathBuf::from("./tunajack.com")),
        ];

        let filtered =
            super::super::filter_status_repositories(repos, &["tunajack.com".to_string()]);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, "tunajack.com");
    }

    #[test]
    fn filters_status_repositories_by_relative_path() {
        let repos = vec![
            ("logger".to_string(), PathBuf::from("./packages/logger")),
            ("frontdesk".to_string(), PathBuf::from("./frontdesk")),
        ];

        let filtered =
            super::super::filter_status_repositories(repos, &["packages/logger".to_string()]);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].0, "logger");
    }

    #[test]
    fn summarizes_worktree_without_counting_untracked_as_unstaged() {
        let (status, parts, details) = summarize_worktree(" M README.md\n?? notes.txt\n", false);

        assert_eq!(status, Status::Dirty);
        assert_eq!(parts, vec!["1 unstaged", "1 untracked"]);
        assert!(details.is_empty());
    }

    #[test]
    fn formats_single_repo_status_details() {
        let details =
            format_status_details(&["M  staged.txt", " M unstaged.txt", "?? new-file.txt"]);

        assert_eq!(
            details,
            vec![
                "    ✓ staged   staged.txt",
                "    ! unstaged unstaged.txt",
                "    · untracked new-file.txt",
            ]
        );
    }

    #[test]
    fn final_status_report_is_attributable_exclusive_and_filter_aware() {
        let entries = vec![
            FleetStatusEntry {
                repository: "healthy".to_string(),
                path: PathBuf::from("healthy"),
                status: FleetStatus {
                    status: Status::Synced,
                    worktree_status: Status::Synced,
                    message: "branch main | clean | synced with origin/main".to_string(),
                    upstream: UpstreamSummary::Remote {
                        message: "synced with origin/main".to_string(),
                        ahead: 0,
                        behind: 0,
                    },
                    failure: None,
                },
            },
            FleetStatusEntry {
                repository: "dirty".to_string(),
                path: PathBuf::from("dirty"),
                status: FleetStatus {
                    status: Status::Dirty,
                    worktree_status: Status::Dirty,
                    message: "branch main | 2 unstaged".to_string(),
                    upstream: UpstreamSummary::NoUpstream,
                    failure: None,
                },
            },
            FleetStatusEntry {
                repository: "broken".to_string(),
                path: PathBuf::from("broken"),
                status: FleetStatus {
                    status: Status::StagingError,
                    worktree_status: Status::StagingError,
                    message: "status failed: permission denied".to_string(),
                    upstream: UpstreamSummary::Unknown,
                    failure: None,
                },
            },
        ];

        let report =
            generate_status_report(&entries, StatusFilters::default(), Duration::from_secs(2));
        assert!(report.contains("repos status"));
        assert!(report.contains("Healthy         1"));
        assert!(report.contains("Needs work      1"));
        assert!(report.contains("Failed          1"));
        assert!(report.contains("Checked         3"));
        assert!(report.contains("path: ./healthy"));
        assert!(report.contains("path: ./dirty"));
        assert!(report.contains("next: commit or stash"));
        assert!(report.contains("path: ./broken"));
        assert!(report.contains("next: inspect the reported status failure"));

        let filtered = generate_status_report(
            &entries,
            StatusFilters {
                dirty: true,
                ..StatusFilters::default()
            },
            Duration::ZERO,
        );
        assert!(filtered.contains("Shown           1"));
        assert!(filtered.contains("path: ./dirty"));
        assert!(!filtered.contains("path: ./healthy"));
        assert!(!filtered.contains("path: ./broken"));
    }

    #[test]
    fn ahead_and_behind_repositories_need_action() {
        for (ahead, behind, next) in [
            (1, 0, "run `repos push`"),
            (0, 2, "run `repos pull`"),
            (1, 2, "run `repos sync` or resolve the divergence manually"),
        ] {
            let status = FleetStatus {
                status: Status::Synced,
                worktree_status: Status::Synced,
                message: String::new(),
                upstream: UpstreamSummary::Remote {
                    message: String::new(),
                    ahead,
                    behind,
                },
                failure: None,
            };

            assert!(status.needs_work());
            assert_eq!(
                status.next_action(&PathBuf::from("repo")).as_deref(),
                Some(next)
            );
        }
    }

    #[test]
    fn dirty_filter_preserves_worktree_state_when_remote_refresh_fails() {
        let status = FleetStatus {
            status: Status::Error,
            worktree_status: Status::Dirty,
            message: "branch main | 1 unstaged | network error during fetch".to_string(),
            upstream: UpstreamSummary::Unknown,
            failure: None,
        };

        assert!(status.dirty());
        assert!(status.failed());
        assert!(status.matches_filters(StatusFilters {
            dirty: true,
            ..StatusFilters::default()
        }));
    }
}
