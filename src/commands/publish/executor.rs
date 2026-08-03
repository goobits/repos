use super::planner::PackageToPublish;
use crate::core::{
    clean_error_message, create_processing_context, create_progress_bar, format_relative_repo_path,
    truncate_text,
};
use crate::git::create_and_push_tag;
use crate::package::PublishStatus;
use crate::utils::compare_repository_locations;
use futures::stream::{FuturesUnordered, StreamExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const PUBLISHING_MESSAGE: &str = "publishing...";
const RESET: &str = "\x1b[0m";
const BOLD_BLUE: &str = "\x1b[1;38;5;75m";
const BOLD_PURPLE: &str = "\x1b[1;38;5;141m";
const GREEN: &str = "\x1b[1;38;5;114m";
const YELLOW: &str = "\x1b[1;38;5;221m";
const RED: &str = "\x1b[1;38;5;203m";
const DIM: &str = "\x1b[2m";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublishOutcomeKind {
    Published,
    AlreadyPublished,
    Failed,
}

#[derive(Debug)]
struct PublishOutcome {
    package: String,
    path: PathBuf,
    kind: PublishOutcomeKind,
    message: String,
}

/// Statistics for publish operations
#[derive(Default)]
struct PublishStatistics {
    outcomes: Vec<PublishOutcome>,
}

impl PublishStatistics {
    fn update(&mut self, status: &PublishStatus, package: &str, path: &Path, message: &str) {
        let kind = match status {
            PublishStatus::Published => PublishOutcomeKind::Published,
            PublishStatus::AlreadyPublished => PublishOutcomeKind::AlreadyPublished,
            PublishStatus::Error => PublishOutcomeKind::Failed,
            PublishStatus::Skipped | PublishStatus::DryRunOk => return,
        };
        self.outcomes.push(PublishOutcome {
            package: package.to_string(),
            path: path.to_path_buf(),
            kind,
            message: clean_error_message(message),
        });
    }

    fn count(&self, kind: PublishOutcomeKind) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| outcome.kind == kind)
            .count()
    }

    fn generate_live_summary(&self, total: usize) -> String {
        let mut parts = Vec::new();
        let published = self.count(PublishOutcomeKind::Published);
        let already_published = self.count(PublishOutcomeKind::AlreadyPublished);
        let failed = self.count(PublishOutcomeKind::Failed);

        if published > 0 {
            parts.push(format!("✅ {published} published"));
        }

        if already_published > 0 {
            parts.push(format!("⚠️  {already_published} already published"));
        }

        if failed > 0 {
            parts.push(format!("❌ {failed} failed"));
        }

        let remaining = total.saturating_sub(self.outcomes.len());
        parts.push(format!("↳ publishing {remaining} remaining"));

        parts.join("  ")
    }

    fn generate_report(&self, duration: Duration) -> String {
        let mut outcomes = self.outcomes.iter().collect::<Vec<_>>();
        outcomes.sort_by(|left, right| {
            compare_repository_locations(&left.path, &left.package, &right.path, &right.package)
        });

        let published = self.count(PublishOutcomeKind::Published);
        let already_published = self.count(PublishOutcomeKind::AlreadyPublished);
        let failed = self.count(PublishOutcomeKind::Failed);
        let mut lines = vec![
            format!("{BOLD_BLUE}repos publish{RESET}"),
            format!(
                "{GREEN}✓{RESET} Completed in {:.1}s",
                duration.as_secs_f64()
            ),
            String::new(),
            format!("{BOLD_PURPLE}▌ Summary{RESET}"),
            format!("  {GREEN}✓{RESET} {:<18}{published}", "Published"),
            format!(
                "  {GREEN}✓{RESET} {:<18}{already_published}",
                "Already published"
            ),
        ];
        if failed > 0 {
            lines.push(format!("  {RED}!{RESET} {:<18}{failed}", "Failed"));
        }
        lines.push(format!(
            "  {DIM}·{RESET} {:<18}{}",
            "Checked",
            self.outcomes.len()
        ));

        append_outcome_section(
            &mut lines,
            &outcomes,
            PublishOutcomeKind::Published,
            "Published",
            GREEN,
            "✓",
        );
        append_outcome_section(
            &mut lines,
            &outcomes,
            PublishOutcomeKind::AlreadyPublished,
            "Already published",
            YELLOW,
            "·",
        );
        append_outcome_section(
            &mut lines,
            &outcomes,
            PublishOutcomeKind::Failed,
            "Failed",
            RED,
            "!",
        );

        lines.join("\n")
    }
}

fn append_outcome_section(
    lines: &mut Vec<String>,
    outcomes: &[&PublishOutcome],
    kind: PublishOutcomeKind,
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
            truncate_text(&outcome.package, 24),
            outcome.message
        ));
        lines.push(format!(
            "    {DIM}↳ path: {}{RESET}",
            format_relative_repo_path(&outcome.path.to_string_lossy())
        ));
        if kind == PublishOutcomeKind::Failed {
            lines.push(format!(
                "    {DIM}↳ next: inspect registry credentials/version, then retry `repos publish {}`{RESET}",
                outcome.package
            ));
        }
    }
}

pub async fn execute_publish(
    packages: Vec<PackageToPublish>,
    tag: bool,
    start_time: std::time::Instant,
) -> anyhow::Result<()> {
    if packages.is_empty() {
        return Ok(());
    }

    let total_packages = packages.len();

    // Create processing context
    let repos_for_context: Vec<(String, PathBuf)> = packages
        .iter()
        .map(|p| (p.name.clone(), p.path.clone()))
        .collect();

    let context = create_processing_context(
        std::sync::Arc::new(repos_for_context),
        start_time,
        crate::core::GIT_CONCURRENT_CAP,
    )?;

    let mut futures = FuturesUnordered::new();
    let statistics = Arc::new(Mutex::new(PublishStatistics::default()));

    // Create progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in context.repositories.iter() {
        let progress_bar =
            create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
        progress_bar.set_message(PUBLISHING_MESSAGE);
        repo_progress_bars.push(progress_bar);
    }

    // Separator and Footer
    let _separator_pb = crate::core::create_separator_progress_bar(&context.multi_progress);
    let footer_pb = crate::core::create_footer_progress_bar(&context.multi_progress);
    footer_pb.set_message("Starting...");
    let _separator_pb2 = crate::core::create_separator_progress_bar(&context.multi_progress);

    let max_name_length = context.max_name_length;
    let publish_semaphore = Arc::new(tokio::sync::Semaphore::new(8));

    for ((pkg, progress_bar), _) in packages
        .into_iter()
        .zip(repo_progress_bars)
        .zip(context.repositories.iter())
    {
        let stats_clone = Arc::clone(&statistics);
        let semaphore_clone = Arc::clone(&publish_semaphore);
        let footer_clone = footer_pb.clone();

        let future = async move {
            let _permit = semaphore_clone.acquire().await.expect("Semaphore closed");

            let (success, message) = pkg.manager.publish(&pkg.path, false).await;

            let mut status = if success {
                if message.contains("already") {
                    PublishStatus::AlreadyPublished
                } else {
                    PublishStatus::Published
                }
            } else {
                PublishStatus::Error
            };

            let mut final_message = message.clone();
            if tag && matches!(status, PublishStatus::Published) {
                if let Some(info) = pkg.manager.get_info(&pkg.path).await {
                    let tag_name = format!("v{}", info.version);
                    let (tag_success, tag_message) =
                        create_and_push_tag(&pkg.path, &tag_name).await;
                    if tag_success {
                        final_message = format!("{message}, {tag_message}");
                    } else {
                        status = PublishStatus::Error;
                        final_message = format!("{message}; tag failed: {tag_message}");
                    }
                }
            }

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                pkg.name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<20}   {}", status.text(), final_message));
            progress_bar.finish();

            {
                let mut stats_guard = stats_clone.lock().expect("Mutex poisoned");
                stats_guard.update(&status, &pkg.name, &pkg.path, &final_message);
                footer_clone.set_message(stats_guard.generate_live_summary(total_packages));
            }
        };

        futures.push(future);
    }

    while futures.next().await.is_some() {}
    footer_pb.finish();

    let final_stats = statistics.lock().expect("Mutex poisoned");
    println!("\n{}\n", final_stats.generate_report(start_time.elapsed()));

    let error_count = final_stats.count(PublishOutcomeKind::Failed);
    drop(final_stats);
    if error_count > 0 {
        anyhow::bail!("{error_count} packages failed to publish completely");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_report_names_every_publish_outcome_and_keeps_counts_exclusive() {
        let mut stats = PublishStatistics::default();
        stats.update(
            &PublishStatus::Published,
            "alpha",
            Path::new("alpha"),
            "published v1.2.3",
        );
        stats.update(
            &PublishStatus::AlreadyPublished,
            "beta",
            Path::new("beta"),
            "version already published",
        );
        stats.update(
            &PublishStatus::Error,
            "gamma",
            Path::new("gamma"),
            "registry rejected package",
        );

        let report = stats.generate_report(Duration::from_secs(3));

        assert!(report.contains("repos publish"));
        assert!(report.contains("Published         1"));
        assert!(report.contains("Already published 1"));
        assert!(report.contains("Failed            1"));
        assert!(report.contains("Checked           3"));
        assert!(report.contains("▌ Published"));
        assert!(report.contains("path: ./alpha"));
        assert!(report.contains("▌ Already published"));
        assert!(report.contains("path: ./beta"));
        assert!(report.contains("▌ Failed"));
        assert!(report.contains("path: ./gamma"));
        assert!(report.contains("next: inspect registry credentials/version"));
    }
}
