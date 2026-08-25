//! Live summaries and final reports for fetch, push, and pull operations.

use super::report::*;
use super::*;
use std::time::Duration;

impl SyncStatistics {
    pub fn generate_push_summary(&self, duration: Duration) -> String {
        self.generate_transfer_summary(Transfer::Push, duration)
    }

    pub fn generate_fetch_summary(&self, duration: Duration) -> String {
        self.generate_transfer_summary(Transfer::Fetch, duration)
    }

    pub fn generate_pull_summary(&self, duration: Duration) -> String {
        self.generate_transfer_summary(Transfer::Pull, duration)
    }

    fn generate_transfer_summary(&self, transfer: Transfer, duration: Duration) -> String {
        let duration_secs = duration.as_secs_f64();
        let (transferred_repos, transferred_commits) = self.transfer_counts(transfer);
        let up_to_date = self.up_to_date_count(transfer);
        let verb = transfer.verb();
        let errors = self.error_repos.load(Ordering::Relaxed);
        let skipped = self.skipped_repos.load(Ordering::Relaxed);

        let unit = transfer.unit(transferred_commits);
        format!(
            "✅ Completed in {duration_secs:.1}s • {up_to_date} up to date • {transferred_repos} {verb} ({transferred_commits} {unit}) • {errors} failed • {skipped} skipped"
        )
    }

    pub fn generate_push_live_summary(&self, total_repos: usize) -> String {
        self.generate_transfer_live_summary(Transfer::Push, total_repos)
    }

    pub fn generate_fetch_live_summary(&self, total_repos: usize) -> String {
        self.generate_transfer_live_summary(Transfer::Fetch, total_repos)
    }

    pub fn generate_pull_live_summary(&self, total_repos: usize) -> String {
        self.generate_transfer_live_summary(Transfer::Pull, total_repos)
    }

    fn generate_transfer_live_summary(&self, transfer: Transfer, total_repos: usize) -> String {
        let (transferred_repos, transferred_commits) = self.transfer_counts(transfer);
        let up_to_date = self.up_to_date_count(transfer);
        let marker = transfer.marker();
        let verb = transfer.verb();
        let errors = self.error_repos.load(Ordering::Relaxed);
        let skipped = self.skipped_repos.load(Ordering::Relaxed);
        let follow_up = self.transfer_follow_up_count();
        let unit = transfer.unit(transferred_commits);
        let processed = up_to_date
            .saturating_add(transferred_repos)
            .saturating_add(errors)
            .saturating_add(skipped);
        let remaining = (total_repos as u64).saturating_sub(processed);

        format!(
            "  {GREEN}✓{RESET} {up_to_date} up to date   {GREEN}{marker}{RESET} {transferred_repos} {verb} / {transferred_commits} {unit}   {RED}!{RESET} {errors} failed   {DIM}·{RESET} {skipped} skipped   {YELLOW}~{RESET} {follow_up} follow-up\n  {DIM}↳ scanning {remaining} remaining{RESET}",
        )
    }

    pub fn generate_push_report(&self, duration: Duration, show_changes: bool) -> String {
        self.generate_push_report_with_follow_up(duration, show_changes, 0, &[])
    }

    pub fn generate_fetch_report(&self, duration: Duration) -> String {
        self.generate_transfer_report_with_follow_up(Transfer::Fetch, duration, false, 0, &[])
    }

    pub fn generate_pull_report(&self, duration: Duration, show_changes: bool) -> String {
        self.generate_pull_report_with_follow_up(duration, show_changes, 0, &[])
    }

    pub fn generate_push_report_with_follow_up(
        &self,
        duration: Duration,
        show_changes: bool,
        extra_follow_up_count: usize,
        extra_follow_up_lines: &[String],
    ) -> String {
        self.generate_transfer_report_with_follow_up(
            Transfer::Push,
            duration,
            show_changes,
            extra_follow_up_count,
            extra_follow_up_lines,
        )
    }

    pub fn generate_pull_report_with_follow_up(
        &self,
        duration: Duration,
        show_changes: bool,
        extra_follow_up_count: usize,
        extra_follow_up_lines: &[String],
    ) -> String {
        self.generate_transfer_report_with_follow_up(
            Transfer::Pull,
            duration,
            show_changes,
            extra_follow_up_count,
            extra_follow_up_lines,
        )
    }

    fn generate_transfer_report_with_follow_up(
        &self,
        transfer: Transfer,
        duration: Duration,
        show_changes: bool,
        extra_follow_up_count: usize,
        extra_follow_up_lines: &[String],
    ) -> String {
        let duration_secs = duration.as_secs_f64();
        let (transferred_repos, transferred_commits) = self.transfer_counts(transfer);
        let up_to_date = self.up_to_date_count(transfer);
        let skipped = self.skipped_repos.load(Ordering::Relaxed);
        let errors = self.error_repos.load(Ordering::Relaxed);
        let checked = up_to_date
            .saturating_add(transferred_repos)
            .saturating_add(skipped)
            .saturating_add(errors);

        let mut transferred_details = match transfer {
            Transfer::Fetch => clone_vec(&self.fetched_repo_details, "fetched_repo_details"),
            Transfer::Push => clone_vec(&self.pushed_repo_details, "pushed_repo_details"),
            Transfer::Pull => clone_vec(&self.pulled_repo_details, "pulled_repo_details"),
        };
        let mut failed_repos = clone_vec(&self.failed_repos, "failed_repos");
        let mut outcomes = clone_vec(&self.operation_outcomes, "operation_outcomes");
        let git_failures = clone_failure_map(&self.git_failures);

        transferred_details.sort_by(|left, right| {
            compare_repository_locations(&left.1, &left.0, &right.1, &right.0)
        });
        failed_repos.sort_by(|left, right| {
            compare_repository_locations(&left.1, &left.0, &right.1, &right.0)
        });
        outcomes.sort_by(|left, right| {
            compare_repository_locations(
                &left.path,
                &left.repository,
                &right.path,
                &right.repository,
            )
        });
        let skipped_outcomes = outcomes
            .iter()
            .filter(|outcome| is_transfer_skip(outcome.status))
            .collect::<Vec<_>>();
        let local_follow_up = outcomes
            .iter()
            .filter(|outcome| outcome.has_uncommitted && is_transfer_success(outcome.status))
            .map(|outcome| (outcome.repository.clone(), outcome.path.clone()))
            .collect::<Vec<_>>();
        let follow_up_count = local_follow_up.len() + extra_follow_up_count;
        let mut lines = Vec::new();
        let transfer_label = transfer.label();
        let totals = TransferTotals {
            transferred_repos,
            transferred_commits,
            up_to_date,
            errors,
            skipped,
            follow_up: follow_up_count,
            checked,
        };

        lines.push(format!("{BOLD_BLUE}repos {}{RESET}", transfer.command()));
        lines.push(format!("{GREEN}✓{RESET} Completed in {duration_secs:.1}s"));
        lines.push(String::new());
        lines.push(format!("{BOLD_PURPLE}▌ {transfer_label}{RESET}"));
        if transferred_details.is_empty() {
            lines.push(format!(
                "  {DIM}Nothing {} this run.{RESET}",
                transfer.verb()
            ));
        } else {
            for (repo_name, repo_path, commits) in transferred_details {
                let unit = transfer.unit(commits);
                lines.push(format!(
                    "  {GREEN}✓{RESET} {:24} {:>3} {unit}",
                    truncate_text(&repo_name, 24),
                    commits
                ));
                lines.push(format!(
                    "    {DIM}↳ path: {}{RESET}",
                    format_relative_repo_path(&repo_path)
                ));
            }
        }
        lines.push(String::new());

        let mut attention =
            transfer_failure_attention(transfer, errors, &failed_repos, &git_failures);
        attention.extend(transfer_skip_attention(transfer, &skipped_outcomes));
        attention.extend(local_follow_up.into_iter().map(|(repository, path)| {
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
        if !extra_follow_up_lines.is_empty() {
            lines.push(String::new());
            lines.extend(extra_follow_up_lines.iter().cloned());
            lines.push(String::new());
        }

        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }

        lines.push(String::new());
        append_transfer_summary(&mut lines, transfer, totals);

        lines.join("\n")
    }

    fn transfer_counts(&self, transfer: Transfer) -> (u64, u64) {
        match transfer {
            Transfer::Fetch => (
                self.fetched_repos.load(Ordering::Relaxed),
                self.total_refs_fetched.load(Ordering::Relaxed),
            ),
            Transfer::Push => (
                self.pushed_repos.load(Ordering::Relaxed),
                self.total_commits_pushed.load(Ordering::Relaxed),
            ),
            Transfer::Pull => (
                self.pulled_repos.load(Ordering::Relaxed),
                self.total_commits_pulled.load(Ordering::Relaxed),
            ),
        }
    }

    fn up_to_date_count(&self, transfer: Transfer) -> u64 {
        let synced = self.synced_repos.load(Ordering::Relaxed);
        let (transferred, _) = self.transfer_counts(transfer);
        synced.saturating_sub(transferred)
    }

    fn transfer_follow_up_count(&self) -> u64 {
        clone_vec(&self.operation_outcomes, "operation_outcomes")
            .iter()
            .filter(|outcome| outcome.has_uncommitted && is_transfer_success(outcome.status))
            .count() as u64
    }
}
