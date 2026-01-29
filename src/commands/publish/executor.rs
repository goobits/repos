use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use futures::stream::{FuturesUnordered, StreamExt};
use crate::core::{create_processing_context, create_progress_bar};
use crate::git::create_and_push_tag;
use crate::package::PublishStatus;
use super::planner::PackageToPublish;

const PUBLISHING_MESSAGE: &str = "publishing...";

/// Statistics for publish operations
#[derive(Default)]
struct PublishStatistics {
    published: usize,
    already_published: usize,
    errors: Vec<(String, String)>,
}

impl PublishStatistics {
    fn update(&mut self, status: &PublishStatus, repo_name: &str, message: &str) {
        match status {
            PublishStatus::Published => self.published += 1,
            PublishStatus::AlreadyPublished => self.already_published += 1,
            PublishStatus::Error => self
                .errors
                .push((repo_name.to_string(), message.to_string())),
            _ => {}
        }
    }

    fn generate_summary(&self, _total: usize) -> String {
        let mut parts = Vec::new();

        if self.published > 0 {
            parts.push(format!("✅ {} published", self.published));
        }

        if self.already_published > 0 {
            parts.push(format!("⚠️  {} already published", self.already_published));
        }

        if !self.errors.is_empty() {
            parts.push(format!("❌ {} failed", self.errors.len()));
        }

        if parts.is_empty() {
            "No changes".to_string()
        } else {
            parts.join("  ")
        }
    }
}

pub async fn execute_publish(
    packages: Vec<PackageToPublish>,
    tag: bool,
    start_time: std::time::Instant,
) -> anyhow::Result<()> {
    if packages.is_empty() {
        return Ok(())
    }

    let total_packages = packages.len();
    
    // Create processing context
    let repos_for_context: Vec<(String, PathBuf)> = packages
        .iter()
        .map(|p| (p.name.clone(), p.path.clone()))
        .collect();

    let context = create_processing_context(repos_for_context, start_time, crate::core::GIT_CONCURRENT_CAP)?;

    let mut futures = FuturesUnordered::new();
    let statistics = Arc::new(Mutex::new(PublishStatistics::default()));

    // Create progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in &context.repositories {
        let progress_bar = create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
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

    for ((pkg, progress_bar), _) in packages.into_iter().zip(repo_progress_bars).zip(context.repositories) {
        let stats_clone = Arc::clone(&statistics);
        let semaphore_clone = Arc::clone(&publish_semaphore);
        let footer_clone = footer_pb.clone();

        let future = async move {
            let _permit = semaphore_clone.acquire().await.expect("Semaphore closed");

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

            let mut final_message = message.clone();
            if tag && matches!(status, PublishStatus::Published) {
                if let Some(info) = pkg.manager.get_info(&pkg.path).await {
                    let tag_name = format!("v{}", info.version);
                    let (tag_success, tag_message) = create_and_push_tag(&pkg.path, &tag_name).await;
                    if tag_success {
                        final_message = format!("{message}, {tag_message}");
                    } else {
                        final_message = format!("{message} (tag failed: {tag_message})");
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
                stats_guard.update(&status, &pkg.name, &final_message);
                footer_clone.set_message(stats_guard.generate_summary(total_packages));
            }
        };

        futures.push(future);
    }

    while futures.next().await.is_some() {}
    footer_pb.finish();

    let final_stats = statistics.lock().expect("Mutex poisoned");
    if !final_stats.errors.is_empty() {
        println!("\n{}", "━".repeat(70));
        println!("❌ Failed to publish:\n");
        for (repo, error) in &final_stats.errors {
            println!("  • {repo}: {error}");
        }
        println!("{}", "━".repeat(70));
    }
    println!();

    Ok(())
}
