//! Repository publish command implementation
//!
//! This module handles the publish functionality - discovering repositories
//! with packages and publishing them to their respective registries.

use anyhow::Result;
use std::collections::HashMap;

use crate::core::{
    create_processing_context, init_command, set_terminal_title, set_terminal_title_and_flush,
    NO_REPOS_MESSAGE,
};
use crate::package::{detect_package_manager, get_package_info, publish_package, PublishStatus};

const SCANNING_MESSAGE: &str = "🔍 Scanning for packages...";
const PUBLISHING_MESSAGE: &str = "publishing...";

/// Handles the repository publish command
pub async fn handle_publish_command(
    target_repos: Vec<String>,
    dry_run: bool,
) -> Result<()> {
    // Set terminal title to indicate repos is running
    set_terminal_title("📦 repos");

    let (start_time, mut repos) = init_command(SCANNING_MESSAGE);

    if repos.is_empty() {
        println!("\r{}", NO_REPOS_MESSAGE);
        set_terminal_title_and_flush("✅ repos");
        return Ok(());
    }

    // Filter repositories if specific targets were requested
    if !target_repos.is_empty() {
        repos.retain(|(name, _)| {
            target_repos.iter().any(|target| name.contains(target))
        });
    }

    // Filter out repositories without packages
    let mut packages_to_publish = Vec::new();
    for (name, path) in repos {
        if let Some(manager) = detect_package_manager(&path) {
            packages_to_publish.push((name, path, manager));
        }
    }

    if packages_to_publish.is_empty() {
        if target_repos.is_empty() {
            println!("\r📦 No packages found in any repository\n");
        } else {
            println!("\r📦 No packages found matching: {}\n", target_repos.join(", "));
        }
        set_terminal_title_and_flush("✅ repos");
        return Ok(());
    }

    // If dry run, show what would be published
    if dry_run {
        println!("\r📦 Found {} packages (dry-run mode)\n", packages_to_publish.len());

        for (name, path, manager) in &packages_to_publish {
            if let Some(info) = get_package_info(path).await {
                println!(
                    "  {} {:<30} ({:<7}) v{}",
                    manager.icon(),
                    name,
                    manager.name(),
                    info.version
                );
            } else {
                println!(
                    "  {} {:<30} ({:<7}) version unknown",
                    manager.icon(),
                    name,
                    manager.name()
                );
            }
        }

        println!("\nWould publish {} packages (dry-run - nothing published)\n", packages_to_publish.len());
        set_terminal_title_and_flush("✅ repos");
        return Ok(());
    }

    // Show what will be published
    let total_packages = packages_to_publish.len();
    let package_word = if total_packages == 1 {
        "package"
    } else {
        "packages"
    };

    print!(
        "\r📦 Publishing {} {}                    \n",
        total_packages, package_word
    );
    println!();

    // Create processing context
    let repos_for_context: Vec<(String, std::path::PathBuf)> = packages_to_publish
        .iter()
        .map(|(name, path, _)| (name.clone(), path.clone()))
        .collect();

    let context = match create_processing_context(repos_for_context, start_time) {
        Ok(context) => context,
        Err(e) => {
            set_terminal_title_and_flush("✅ repos");
            return Err(e);
        }
    };

    // Process all packages concurrently
    process_publish_repositories(context, packages_to_publish).await;

    // Set terminal title to green checkbox to indicate completion
    set_terminal_title_and_flush("✅ repos");

    Ok(())
}

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
            PublishStatus::Error => self.errors.push((repo_name.to_string(), message.to_string())),
            _ => {}
        }
    }

    fn generate_summary(&self, total: usize) -> String {
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

/// Processes all packages concurrently for publishing
async fn process_publish_repositories(
    context: crate::core::ProcessingContext,
    packages: Vec<(String, std::path::PathBuf, crate::package::PackageManager)>,
) {
    use crate::core::{create_progress_bar};
    use futures::stream::{FuturesUnordered, StreamExt};
    use std::sync::{Arc, Mutex};

    let mut futures = FuturesUnordered::new();

    // Create statistics tracker
    let statistics = Arc::new(Mutex::new(PublishStatistics::default()));

    // First, create all repository progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in &context.repositories {
        let progress_bar =
            create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
        progress_bar.set_message(PUBLISHING_MESSAGE);
        repo_progress_bars.push(progress_bar);
    }

    // Add a blank line before the footer
    let _separator_pb = crate::core::create_separator_progress_bar(&context.multi_progress);

    // Create the footer progress bar
    let footer_pb = crate::core::create_footer_progress_bar(&context.multi_progress);

    // Initial footer display
    footer_pb.set_message("Starting...");

    // Add another blank line after the footer
    let _separator_pb2 = crate::core::create_separator_progress_bar(&context.multi_progress);

    // Extract values we need in the async closures
    let max_name_length = context.max_name_length;
    let total_packages = packages.len();

    // Use a lower concurrency for publishing (3 concurrent max)
    let publish_semaphore = Arc::new(tokio::sync::Semaphore::new(3));

    for (((repo_name, repo_path, manager), progress_bar), _) in
        packages.into_iter().zip(repo_progress_bars).zip(context.repositories)
    {
        let stats_clone = Arc::clone(&statistics);
        let semaphore_clone = Arc::clone(&publish_semaphore);
        let footer_clone = footer_pb.clone();

        let future = async move {
            let _permit = semaphore_clone.acquire().await.unwrap();

            let (success, message) = publish_package(&repo_path, &manager, false).await;

            let status = if success {
                if message.contains("already") {
                    PublishStatus::AlreadyPublished
                } else {
                    PublishStatus::Published
                }
            } else {
                PublishStatus::Error
            };

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                repo_name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<20}   {}", status.text(), message));
            progress_bar.finish();

            // Update statistics
            {
                let mut stats_guard = stats_clone.lock().unwrap();
                stats_guard.update(&status, &repo_name, &message);

                // Update the footer summary
                let summary = stats_guard.generate_summary(total_packages);
                footer_clone.set_message(summary);
            }
        };

        futures.push(future);
    }

    // Wait for all publish operations to complete
    while futures.next().await.is_some() {}

    // Finish the footer progress bar
    footer_pb.finish();

    // Print detailed error summary if there are errors
    let final_stats = statistics.lock().unwrap();
    if !final_stats.errors.is_empty() {
        println!("\n{}", "━".repeat(70));
        println!("❌ Failed to publish:\n");
        for (repo, error) in &final_stats.errors {
            println!("  • {}: {}", repo, error);
        }
        println!("{}", "━".repeat(70));
    }

    // Add final spacing
    println!();
}
