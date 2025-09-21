//! Repository hygiene checking for detecting improperly committed files
//!
//! Repository hygiene checking functionality for detecting improperly committed files.
//!
//! This module provides:
//! - Detection of files that violate .gitignore patterns
//! - Identification of universal bad patterns (node_modules, vendor, etc.)
//! - Large file detection in git history
//! - Statistics tracking and reporting
//! - Concurrent processing with progress tracking

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;

use crate::core::{create_progress_bar, GenericProcessingContext};
use crate::utils::shorten_path;

// =====================================================================================
// Hygiene checking constants and types
// =====================================================================================

// Universal patterns that should never be committed to git
const UNIVERSAL_BAD_PATTERNS: &[&str] = &[
    "node_modules/",
    "vendor/",
    "dist/",
    "build/",
    "target/debug/",
    "target/release/",
    ".env",
    "*.log",
    ".DS_Store",
    "Thumbs.db",
    "*.tmp",
    "*.cache",
    "__pycache__/",
    ".venv/",
    ".idea/",
    ".vscode/settings.json",
    "*.key",
    "*.pem",
    "*.p12",
    "*.jks",
];

// Large file threshold in bytes (1MB)
const LARGE_FILE_THRESHOLD: u64 = 1_048_576;

/// Status for hygiene check results
#[derive(Clone, Debug)]
pub enum HygieneStatus {
    Clean,      // No hygiene violations found
    Violations, // Hygiene violations detected
    Error,      // Scan failed
}

impl HygieneStatus {
    fn symbol(&self) -> &str {
        match self {
            HygieneStatus::Clean => "🟢",
            HygieneStatus::Violations => "🟡",
            HygieneStatus::Error => "🟠",
        }
    }

    fn text(&self) -> &str {
        match self {
            HygieneStatus::Clean => "clean",
            HygieneStatus::Violations => "violations",
            HygieneStatus::Error => "failed",
        }
    }
}

/// Type of hygiene violation
#[derive(Clone, Debug)]
pub enum ViolationType {
    GitignoreViolation,  // File tracked but matches .gitignore
    UniversalBadPattern, // File matches universal bad patterns
    LargeFile,           // File is unusually large
}

/// Individual hygiene violation
#[derive(Clone, Debug)]
pub struct HygieneViolation {
    pub file_path: String,
    pub violation_type: ViolationType,
    pub size_bytes: Option<u64>,
}

/// Statistics for hygiene scanning results
#[derive(Clone, Default)]
pub struct HygieneStatistics {
    clean_repos: u32,
    repos_with_violations: u32,
    total_violations: u32,
    gitignore_violations: u32,
    universal_violations: u32,
    large_files: u32,
    error_repos: u32,
    failed_repos: Vec<(String, String, String)>, // (repo_name, repo_path, error_message)
    violation_repos: Vec<(String, String, Vec<HygieneViolation>)>, // (repo_name, repo_path, violations)
}

impl HygieneStatistics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the violation repositories for fix operations
    pub fn get_violation_repos(&self) -> Vec<(String, String, Vec<HygieneViolation>)> {
        self.violation_repos.clone()
    }

    pub fn update(
        &mut self,
        repo_name: &str,
        repo_path: &str,
        status: &HygieneStatus,
        message: &str,
        violations: Vec<HygieneViolation>,
    ) {
        match status {
            HygieneStatus::Clean => self.clean_repos += 1,
            HygieneStatus::Violations => {
                self.repos_with_violations += 1;
                let violation_count = violations.len() as u32;
                self.total_violations += violation_count;

                // Count by type
                for violation in &violations {
                    match violation.violation_type {
                        ViolationType::GitignoreViolation => self.gitignore_violations += 1,
                        ViolationType::UniversalBadPattern => self.universal_violations += 1,
                        ViolationType::LargeFile => self.large_files += 1,
                    }
                }

                self.violation_repos.push((
                    repo_name.to_string(),
                    repo_path.to_string(),
                    violations,
                ));
            }
            HygieneStatus::Error => {
                self.error_repos += 1;
                self.failed_repos.push((
                    repo_name.to_string(),
                    repo_path.to_string(),
                    message.to_string(),
                ));
            }
        }
    }

    pub fn generate_summary(&self, _total_repos: usize, duration: Duration) -> String {
        let duration_secs = duration.as_secs_f64();

        if self.error_repos > 0 {
            format!(
                "✅ Completed in {:.1}s • {} clean • {} with violations • {} failed",
                duration_secs, self.clean_repos, self.repos_with_violations, self.error_repos
            )
        } else {
            format!(
                "✅ Completed in {:.1}s • {} clean • {} with violations",
                duration_secs, self.clean_repos, self.repos_with_violations
            )
        }
    }

    pub fn generate_detailed_summary(&self) -> String {
        let mut lines = Vec::new();

        // Repos with violations get priority
        if !self.violation_repos.is_empty() {
            lines.push(format!(
                "🟡 HYGIENE VIOLATIONS ({})",
                self.violation_repos.len()
            ));
            for (i, (repo_name, repo_path, violations)) in self.violation_repos.iter().enumerate() {
                let tree_char = if i == self.violation_repos.len() - 1 {
                    "└─"
                } else {
                    "├─"
                };
                let short_path = shorten_path(repo_path, 30);
                let violation_summary = format!(
                    "{} violations ({} gitignore, {} patterns, {} large)",
                    violations.len(),
                    violations
                        .iter()
                        .filter(|v| matches!(v.violation_type, ViolationType::GitignoreViolation))
                        .count(),
                    violations
                        .iter()
                        .filter(|v| matches!(v.violation_type, ViolationType::UniversalBadPattern))
                        .count(),
                    violations
                        .iter()
                        .filter(|v| matches!(v.violation_type, ViolationType::LargeFile))
                        .count()
                );
                lines.push(format!(
                    "   {} {:20} {:30} # {}",
                    tree_char, repo_name, short_path, violation_summary
                ));
            }
            lines.push(String::new()); // Add blank line
        }

        // Failed repos
        if !self.failed_repos.is_empty() {
            lines.push(format!(
                "🟠 FAILED HYGIENE SCANS ({})",
                self.failed_repos.len()
            ));
            for (i, (repo_name, repo_path, error)) in self.failed_repos.iter().enumerate() {
                let tree_char = if i == self.failed_repos.len() - 1 {
                    "└─"
                } else {
                    "├─"
                };
                let short_path = shorten_path(repo_path, 30);
                lines.push(format!(
                    "   {} {:20} {:30} # {}",
                    tree_char, repo_name, short_path, error
                ));
            }
        }

        // Remove trailing blank line if it exists
        if lines.last() == Some(&String::new()) {
            lines.pop();
        }

        lines.join("\n")
    }
}

// =====================================================================================
// Hygiene checking functions
// =====================================================================================

/// Checks for gitignore violations using git ls-files
async fn check_gitignore_violations(repo_path: &Path) -> Result<Vec<HygieneViolation>> {
    let output = Command::new("git")
        .arg("ls-files")
        .arg("-i")
        .arg("-c")
        .arg("--exclude-standard")
        .current_dir(repo_path)
        .output()
        .await?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut violations = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if !line.is_empty() {
            violations.push(HygieneViolation {
                file_path: line.to_string(),
                violation_type: ViolationType::GitignoreViolation,
                size_bytes: None,
            });
        }
    }

    Ok(violations)
}

/// Checks for universal bad patterns in tracked files
async fn check_universal_patterns(repo_path: &Path) -> Result<Vec<HygieneViolation>> {
    let output = Command::new("git")
        .arg("ls-files")
        .current_dir(repo_path)
        .output()
        .await?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut violations = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Check against universal bad patterns
        for pattern in UNIVERSAL_BAD_PATTERNS {
            let pattern_matches = if pattern.ends_with('/') {
                line.starts_with(pattern) || line.contains(&format!("/{}", pattern))
            } else if pattern.starts_with("*.") {
                let extension = &pattern[1..]; // Remove *
                line.ends_with(extension)
            } else {
                line == *pattern || line.contains(pattern)
            };

            if pattern_matches {
                violations.push(HygieneViolation {
                    file_path: line.to_string(),
                    violation_type: ViolationType::UniversalBadPattern,
                    size_bytes: None,
                });
                break; // Only report each file once
            }
        }
    }

    Ok(violations)
}

/// Checks for large files in git history
async fn check_large_files(repo_path: &Path) -> Result<Vec<HygieneViolation>> {
    let output = Command::new("git")
        .args(["rev-list", "--objects", "--all"])
        .current_dir(repo_path)
        .output()
        .await?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let objects_output = String::from_utf8_lossy(&output.stdout);
    let mut violations = Vec::new();

    // Process in batches to avoid command line length limits
    let objects: Vec<&str> = objects_output.lines().collect();
    for chunk in objects.chunks(100) {
        let batch_input = chunk.join("\n");

        let cat_file_output = Command::new("git")
            .args(["cat-file", "--batch-check=%(objectsize) %(rest)"])
            .current_dir(repo_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let mut child = cat_file_output;
        if let Some(stdin) = child.stdin.as_mut() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(batch_input.as_bytes()).await?;
            stdin.shutdown().await?;
        }

        let output = child.wait_with_output().await?;
        let stdout = String::from_utf8_lossy(&output.stdout);

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                if let Ok(size) = parts[0].parse::<u64>() {
                    if size > LARGE_FILE_THRESHOLD {
                        let file_path = parts[2..].join(" ");
                        if !file_path.is_empty() {
                            violations.push(HygieneViolation {
                                file_path,
                                violation_type: ViolationType::LargeFile,
                                size_bytes: Some(size),
                            });
                        }
                    }
                }
            }
        }
    }

    // Sort by size (largest first) and limit to top 10
    violations.sort_by(|a, b| b.size_bytes.unwrap_or(0).cmp(&a.size_bytes.unwrap_or(0)));
    violations.truncate(10);

    Ok(violations)
}

/// Scans a repository for hygiene violations
async fn check_repo_hygiene(repo_path: &Path) -> (HygieneStatus, String, Vec<HygieneViolation>) {
    let mut all_violations = Vec::new();

    // Check gitignore violations
    match check_gitignore_violations(repo_path).await {
        Ok(mut violations) => all_violations.append(&mut violations),
        Err(e) => {
            return (
                HygieneStatus::Error,
                format!("gitignore check failed: {}", e),
                Vec::new(),
            );
        }
    }

    // Check universal bad patterns
    match check_universal_patterns(repo_path).await {
        Ok(mut violations) => all_violations.append(&mut violations),
        Err(e) => {
            return (
                HygieneStatus::Error,
                format!("pattern check failed: {}", e),
                Vec::new(),
            );
        }
    }

    // Check large files
    match check_large_files(repo_path).await {
        Ok(mut violations) => all_violations.append(&mut violations),
        Err(e) => {
            return (
                HygieneStatus::Error,
                format!("large file check failed: {}", e),
                Vec::new(),
            );
        }
    }

    if all_violations.is_empty() {
        (
            HygieneStatus::Clean,
            "no violations found".to_string(),
            Vec::new(),
        )
    } else {
        let message = format!("{} violations found", all_violations.len());
        (HygieneStatus::Violations, message, all_violations)
    }
}

/// Helper function to safely acquire a semaphore permit
async fn acquire_semaphore_permit(
    semaphore: &tokio::sync::Semaphore,
) -> tokio::sync::SemaphorePermit<'_> {
    semaphore
        .acquire()
        .await
        .expect("Failed to acquire semaphore permit for concurrent hygiene operations")
}

/// Processes all repositories concurrently for hygiene checking
pub async fn process_hygiene_repositories(context: GenericProcessingContext<HygieneStatistics>) {
    use futures::stream::{FuturesUnordered, StreamExt};

    let mut futures = FuturesUnordered::new();

    // Create all repository progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in &context.repositories {
        let progress_bar =
            create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
        progress_bar.set_message("checking hygiene...");
        repo_progress_bars.push(progress_bar);
    }

    // Add a blank line before the footer
    let separator_pb = context.multi_progress.add(ProgressBar::new(0));
    separator_pb.set_style(ProgressStyle::default_bar().template(" ").unwrap());
    separator_pb.finish();

    // Create the footer progress bar
    let footer_pb = context.multi_progress.add(ProgressBar::new(0));
    let footer_style = ProgressStyle::default_bar()
        .template("{wide_msg}")
        .expect("Failed to create footer progress style");
    footer_pb.set_style(footer_style);

    // Initial footer display
    let initial_stats = HygieneStatistics::new();
    let initial_summary =
        initial_stats.generate_summary(context.total_repos, context.start_time.elapsed());
    footer_pb.set_message(initial_summary);

    // Add another blank line after the footer
    let separator_pb2 = context.multi_progress.add(ProgressBar::new(0));
    separator_pb2.set_style(ProgressStyle::default_bar().template(" ").unwrap());
    separator_pb2.finish();

    // Extract values we need in the async closures before moving context.repositories
    let max_name_length = context.max_name_length;
    let start_time = context.start_time;
    let total_repos = context.total_repos;

    for ((repo_name, repo_path), progress_bar) in
        context.repositories.into_iter().zip(repo_progress_bars)
    {
        let stats_clone = Arc::clone(&context.statistics);
        let semaphore_clone = Arc::clone(&context.semaphore);
        let footer_clone = footer_pb.clone();

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore_clone).await;

            let (status, message, violations) = check_repo_hygiene(&repo_path).await;

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                repo_name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<10}   {}", status.text(), message));
            progress_bar.finish();

            // Update statistics
            let mut stats_guard = stats_clone.lock().expect("Failed to acquire stats lock");
            let repo_path_str = repo_path.to_string_lossy();
            stats_guard.update(&repo_name, &repo_path_str, &status, &message, violations);

            // Update the footer summary after each repository completes
            let duration = start_time.elapsed();
            let summary = stats_guard.generate_summary(total_repos, duration);
            footer_clone.set_message(summary);
        };

        futures.push(future);
    }

    // Wait for all repository operations to complete
    while futures.next().await.is_some() {}

    // Finish the footer progress bar
    footer_pb.finish();

    // Print the final detailed summary if there are any issues to report
    let final_stats = context
        .statistics
        .lock()
        .expect("Failed to acquire stats lock");
    let detailed_summary = final_stats.generate_detailed_summary();
    if !detailed_summary.is_empty() {
        println!("\n{}", "━".repeat(70));
        println!("{}", detailed_summary);
        println!("{}", "━".repeat(70));
    }

    // Add final spacing
    println!();
}
