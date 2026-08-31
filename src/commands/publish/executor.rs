use super::planner::PackageToPublish;
use crate::core::{
    clean_error_message, create_processing_context, create_progress_bar, format_relative_repo_path,
    truncate_text,
};
use crate::git::create_and_push_tag;
use crate::package::PublishStatus;
use crate::utils::compare_repository_locations;
use futures::stream::{FuturesUnordered, StreamExt};
use std::collections::{HashMap, HashSet};
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

fn normalize_package_name(manager: &str, name: &str) -> String {
    let name = if manager == "python" {
        name.split(|character: char| {
            character.is_whitespace()
                || matches!(character, '<' | '>' | '=' | '!' | '~' | ';' | '[' | '(')
        })
        .next()
        .unwrap_or(name)
    } else {
        name
    };
    match manager {
        "cargo" | "python" => name.to_ascii_lowercase().replace(['_', '.'], "-"),
        _ => name.to_ascii_lowercase(),
    }
}

fn publish_dependencies(packages: &[PackageToPublish]) -> anyhow::Result<Vec<Vec<usize>>> {
    let mut identities = HashMap::new();
    for (index, package) in packages.iter().enumerate() {
        let identity = (
            package.manager.name().to_string(),
            normalize_package_name(package.manager.name(), &package.package_name),
        );
        if let Some(existing) = identities.insert(identity, index) {
            anyhow::bail!(
                "duplicate package identity in publish plan: {} and {} both publish {}",
                packages[existing].name,
                package.name,
                package.package_name
            );
        }
    }

    Ok(packages
        .iter()
        .map(|package| {
            package
                .dependencies
                .iter()
                .filter_map(|dependency| {
                    identities
                        .get(&(
                            package.manager.name().to_string(),
                            normalize_package_name(package.manager.name(), dependency),
                        ))
                        .copied()
                })
                .collect::<HashSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        })
        .collect())
}

fn publish_waves(dependencies: &[Vec<usize>]) -> anyhow::Result<Vec<Vec<usize>>> {
    let mut remaining = (0..dependencies.len()).collect::<HashSet<_>>();
    let mut published = HashSet::new();
    let mut waves = Vec::new();
    while !remaining.is_empty() {
        let mut wave = remaining
            .iter()
            .copied()
            .filter(|index| {
                dependencies[*index]
                    .iter()
                    .all(|dependency| published.contains(dependency))
            })
            .collect::<Vec<_>>();
        wave.sort_unstable();
        if wave.is_empty() {
            anyhow::bail!("package dependency cycle detected in publish plan");
        }
        for index in &wave {
            remaining.remove(index);
            published.insert(*index);
        }
        waves.push(wave);
    }
    Ok(waves)
}

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
        let message = if message.contains("registry outcome is unknown") {
            "publish timed out; registry outcome is unknown—check the registry before retrying"
                .to_string()
        } else {
            clean_error_message(message)
        };
        self.outcomes.push(PublishOutcome {
            package: package.to_string(),
            path: path.to_path_buf(),
            kind,
            message,
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
            let next = if outcome.message.contains("registry outcome is unknown") {
                format!(
                    "check the registry for {} before retrying; the publish may have completed",
                    outcome.package
                )
            } else {
                format!(
                    "inspect registry credentials/version, then retry `repos publish {}`",
                    outcome.package
                )
            };
            lines.push(format!("    {DIM}↳ next: {next}{RESET}"));
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
    let dependencies = publish_dependencies(&packages)?;
    let waves = publish_waves(&dependencies)?;

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
    let mut completed = vec![None; packages.len()];

    for wave in waves {
        let mut futures = FuturesUnordered::new();
        for index in wave {
            let pkg = &packages[index];
            let progress_bar = repo_progress_bars[index].clone();
            let blocked_dependencies = dependencies[index]
                .iter()
                .filter(|dependency| completed[**dependency] == Some(false))
                .map(|dependency| packages[*dependency].package_name.clone())
                .collect::<Vec<_>>();
            let stats_clone = Arc::clone(&statistics);
            let semaphore_clone = Arc::clone(&publish_semaphore);
            let footer_clone = footer_pb.clone();

            futures.push(async move {
                let permit = semaphore_clone.acquire().await.expect("Semaphore closed");

                let (mut status, mut final_message) = if blocked_dependencies.is_empty() {
                    let (success, message) = pkg.manager.publish(&pkg.path, false).await;
                    let status = if success {
                        if message.contains("already") {
                            PublishStatus::AlreadyPublished
                        } else {
                            PublishStatus::Published
                        }
                    } else {
                        PublishStatus::Error
                    };
                    (status, message)
                } else {
                    (
                        PublishStatus::Error,
                        format!(
                            "dependency publish failed: {}",
                            blocked_dependencies.join(", ")
                        ),
                    )
                };
                let registry_available = !matches!(status, PublishStatus::Error);

                if tag && registry_available {
                    let tag_name = format!("v{}", pkg.version);
                    let (tag_success, tag_message) =
                        create_and_push_tag(&pkg.path, &tag_name).await;
                    if tag_success {
                        final_message = format!("{final_message}, {tag_message}");
                    } else {
                        status = PublishStatus::Error;
                        final_message = format!("{final_message}; tag failed: {tag_message}");
                    }
                }
                drop(permit);

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
                (index, registry_available)
            });
        }

        while let Some((index, success)) = futures.next().await {
            completed[index] = Some(success);
        }
    }
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
    use crate::package::{PackageInfo, PackageManager};
    use async_trait::async_trait;

    struct TestManager;

    #[async_trait]
    impl PackageManager for TestManager {
        fn name(&self) -> &str {
            "cargo"
        }

        fn icon(&self) -> &str {
            ""
        }

        async fn detect(&self, _path: &Path) -> bool {
            true
        }

        async fn get_info(&self, _path: &Path) -> Option<PackageInfo> {
            None
        }

        async fn dependencies(&self, _path: &Path) -> Vec<String> {
            Vec::new()
        }

        async fn publish(&self, _path: &Path, _dry_run: bool) -> (bool, String) {
            (true, "published".to_string())
        }
    }

    fn package(repository: &str, package_name: &str, dependencies: &[&str]) -> PackageToPublish {
        PackageToPublish {
            name: repository.to_string(),
            package_name: package_name.to_string(),
            version: "1.0.0".to_string(),
            dependencies: dependencies
                .iter()
                .map(|dependency| (*dependency).to_string())
                .collect(),
            path: PathBuf::from(repository),
            manager: Arc::new(TestManager),
        }
    }

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

    #[test]
    fn timeout_report_requires_registry_reconciliation_before_retry() {
        let mut stats = PublishStatistics::default();
        stats.update(
            &PublishStatus::Error,
            "alpha",
            Path::new("alpha"),
            "cargo publish timed out; registry outcome is unknown",
        );

        let report = stats.generate_report(Duration::ZERO);

        assert!(report.contains("check the registry for alpha before retrying"));
        assert!(!report.contains("inspect registry credentials/version"));
    }

    #[test]
    fn package_dependencies_publish_in_topological_waves() {
        let dependencies = vec![vec![], vec![0], vec![0], vec![1, 2]];

        assert_eq!(
            publish_waves(&dependencies).unwrap(),
            vec![vec![0], vec![1, 2], vec![3]]
        );
        assert!(publish_waves(&[vec![1], vec![0]]).is_err());
    }

    #[test]
    fn publish_plan_maps_local_dependencies_and_rejects_duplicate_identities() {
        let packages = vec![
            package("core-repo", "core_lib", &[]),
            package("app-repo", "app", &["core-lib"]),
        ];
        let dependencies = publish_dependencies(&packages).unwrap();
        assert_eq!(dependencies, vec![vec![], vec![0]]);
        assert_eq!(
            publish_waves(&dependencies).unwrap(),
            vec![vec![0], vec![1]]
        );

        let duplicates = vec![
            package("first", "shared_name", &[]),
            package("second", "shared-name", &[]),
        ];
        assert!(publish_dependencies(&duplicates).is_err());
    }

    #[test]
    fn package_names_use_ecosystem_normalization() {
        assert_eq!(normalize_package_name("cargo", "my_crate"), "my-crate");
        assert_eq!(
            normalize_package_name("python", "My_Package>=1.0"),
            "my-package"
        );
        assert_eq!(normalize_package_name("npm", "@Scope/Name"), "@scope/name");
    }
}
