//! Repository publish command implementation
//!
//! This module handles the publish functionality - discovering repositories
//! with packages and publishing them to their respective registries.

use anyhow::Result;

use crate::core::{
    create_processing_context, init_command, set_terminal_title, set_terminal_title_and_flush,
    NO_REPOS_MESSAGE, GIT_CONCURRENT_CAP,
};
use crate::git::{has_uncommitted_changes, create_and_push_tag, get_repo_visibility, RepoVisibility};
use crate::package::{detect_package_manager, get_package_info, publish_package, PublishStatus};

const SCANNING_MESSAGE: &str = "🔍 Scanning for packages...";
const PUBLISHING_MESSAGE: &str = "publishing...";

/// Handles the repository publish command
pub async fn handle_publish_command(
    target_repos: Vec<String>,
    dry_run: bool,
    tag: bool,
    allow_dirty: bool,
    all: bool,
    _public_only: bool,
    private_only: bool,
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

    // Determine visibility filter
    // Default behavior: only public repos (unless --all, --public-only, or --private-only is set)
    let filter_visibility = if all {
        None // No filtering
    } else if private_only {
        Some(RepoVisibility::Private)
    } else {
        // Default or --public-only: filter for public repos
        Some(RepoVisibility::Public)
    };

    // Filter by visibility if needed
    if let Some(desired_visibility) = filter_visibility {
        let mut filtered_repos = Vec::new();
        let mut skipped_count = 0;
        let mut unknown_count = 0;

        for (name, path) in repos {
            let visibility = get_repo_visibility(&path).await;

            if visibility == desired_visibility {
                filtered_repos.push((name, path));
            } else if visibility == RepoVisibility::Unknown {
                // Treat unknown as private (fail-safe)
                if desired_visibility == RepoVisibility::Private {
                    filtered_repos.push((name, path));
                } else {
                    skipped_count += 1;
                    unknown_count += 1;
                }
            } else {
                skipped_count += 1;
            }
        }

        repos = filtered_repos;

        // Show filtering feedback
        if skipped_count > 0 {
            let visibility_type = match desired_visibility {
                RepoVisibility::Public => "public",
                RepoVisibility::Private => "private",
                _ => "unknown",
            };

            let mut skip_msg = format!(
                "\r📦 Filtered to {} repos only ({} skipped)",
                visibility_type, skipped_count
            );

            if unknown_count > 0 {
                skip_msg.push_str(&format!(" [{} unknown visibility treated as private]", unknown_count));
            }

            println!("{}\n", skip_msg);
        }
    }

    // Filter out repositories without packages, and check for uncommitted changes
    let mut packages_to_publish = Vec::new();
    let mut dirty_repos = Vec::new();

    for (name, path) in repos {
        if let Some(manager) = detect_package_manager(&path) {
            // Check for uncommitted changes unless --allow-dirty is set or --dry-run
            if !allow_dirty && !dry_run && has_uncommitted_changes(&path).await {
                dirty_repos.push(name.clone());
            }
            packages_to_publish.push((name, path, manager));
        }
    }

    // If there are dirty repos and we're not allowing dirty, stop
    if !dirty_repos.is_empty() && !allow_dirty && !dry_run {
        println!("\r❌ Cannot publish: {} {} uncommitted changes\n",
            dirty_repos.len(),
            if dirty_repos.len() == 1 { "repository has" } else { "repositories have" }
        );
        println!("Repositories with uncommitted changes:");
        for repo in &dirty_repos {
            println!("  • {}", repo);
        }
        println!("\nCommit your changes first, or use --allow-dirty to publish anyway (not recommended).\n");
        set_terminal_title_and_flush("✅ repos");
        return Ok(());
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

    let context = match create_processing_context(repos_for_context, start_time, GIT_CONCURRENT_CAP) {
        Ok(context) => context,
        Err(e) => {
            set_terminal_title_and_flush("✅ repos");
            return Err(e);
        }
    };

    // Process all packages concurrently
    process_publish_repositories(context, packages_to_publish, tag).await;

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

/// Processes all packages concurrently for publishing
async fn process_publish_repositories(
    context: crate::core::ProcessingContext,
    packages: Vec<(String, std::path::PathBuf, crate::package::PackageManager)>,
    tag: bool,
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

    // Use moderate concurrency for publishing to balance speed with registry rate limits
    // Previously hardcoded to 3, now uses 8 to better utilize modern systems
    // Users experiencing rate limits should use a future --jobs flag to limit concurrency
    let publish_semaphore = Arc::new(tokio::sync::Semaphore::new(8));

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

            // If tagging is enabled and publish was successful, create and push tag
            let mut final_message = message.clone();
            if tag && matches!(status, PublishStatus::Published) {
                // Get package info to determine version
                if let Some(info) = get_package_info(&repo_path).await {
                    let tag_name = format!("v{}", info.version);
                    let (tag_success, tag_message) = create_and_push_tag(&repo_path, &tag_name).await;
                    if tag_success {
                        final_message = format!("{}, {}", message, tag_message);
                    } else {
                        final_message = format!("{} (tag failed: {})", message, tag_message);
                    }
                }
            }

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                repo_name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<20}   {}", status.text(), final_message));
            progress_bar.finish();

            // Update statistics
            {
                let mut stats_guard = stats_clone.lock().unwrap();
                stats_guard.update(&status, &repo_name, &final_message);

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
