//! TruffleHog integration for secret scanning across repositories
//!
//! This module provides:
//! - TruffleHog binary installation and management
//! - Secret scanning across discovered repositories
//! - Progress tracking and result reporting
//! - Self-contained implementation with minimal dependencies

use anyhow::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serde::Deserialize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::Command;

use crate::core::{
    find_repos, create_progress_bar, create_progress_style, acquire_stats_lock,
    shorten_path, NO_REPOS_MESSAGE, DEFAULT_CONCURRENT_LIMIT
};

// TruffleHog constants
const TRUFFLEHOG_VERSION: &str = "v3.90.8";
const TRUFFLEHOG_BASE_URL: &str = "https://github.com/trufflesecurity/trufflehog/releases/download";
const TRUFFLE_SCANNING_MESSAGE: &str = "scanning for secrets...";
const TRUFFLE_TIMEOUT_SECS: u64 = 300; // 5 minutes per repository for TruffleHog

/// TruffleHog scanning results for a single finding
#[derive(Deserialize, Debug)]
struct TruffleHogFinding {
    #[serde(rename = "DetectorName")]
    detector_name: String,
    #[serde(rename = "Verified")]
    verified: bool,
    #[serde(rename = "Raw")]
    raw: Option<String>,
    #[serde(rename = "SourceMetadata")]
    source_metadata: Option<serde_json::Value>,
}

/// Status for TruffleHog scan results
#[derive(Clone, Debug)]
enum TruffleStatus {
    Clean,      // No secrets found
    Secrets,    // Secrets found
    Error,      // Scan failed
    Skipped,    // Scan skipped
}

impl TruffleStatus {
    fn symbol(&self) -> &str {
        match self {
            TruffleStatus::Clean => "🟢",
            TruffleStatus::Secrets => "🔴",
            TruffleStatus::Error => "🟠",
            TruffleStatus::Skipped => "🟡",
        }
    }

    fn text(&self) -> &str {
        match self {
            TruffleStatus::Clean => "clean",
            TruffleStatus::Secrets => "secrets",
            TruffleStatus::Error => "failed",
            TruffleStatus::Skipped => "skipped",
        }
    }
}

/// Statistics for TruffleHog scanning results
#[derive(Clone, Default)]
struct TruffleStatistics {
    clean_repos: u32,
    repos_with_secrets: u32,
    total_secrets: u32,
    verified_secrets: u32,
    error_repos: u32,
    failed_repos: Vec<(String, String, String)>,  // (repo_name, repo_path, error_message)
    secret_repos: Vec<(String, String, u32, u32)>, // (repo_name, repo_path, total_secrets, verified_secrets)
}

impl TruffleStatistics {
    fn new() -> Self {
        Self::default()
    }

    fn update(&mut self, repo_name: &str, repo_path: &str, status: &TruffleStatus, message: &str, secrets: u32, verified: u32) {
        match status {
            TruffleStatus::Clean => self.clean_repos += 1,
            TruffleStatus::Secrets => {
                self.repos_with_secrets += 1;
                self.total_secrets += secrets;
                self.verified_secrets += verified;
                self.secret_repos.push((repo_name.to_string(), repo_path.to_string(), secrets, verified));
            }
            TruffleStatus::Error => {
                self.error_repos += 1;
                self.failed_repos.push((repo_name.to_string(), repo_path.to_string(), message.to_string()));
            }
            TruffleStatus::Skipped => {
                // Don't count skipped repos in totals
            }
        }
    }

    fn generate_summary(&self, _total_repos: usize, duration: Duration) -> String {
        let duration_secs = duration.as_secs_f64();

        if self.error_repos > 0 {
            format!("✅ Completed in {:.1}s • {} clean • {} with secrets • {} failed",
                duration_secs, self.clean_repos, self.repos_with_secrets, self.error_repos)
        } else {
            format!("✅ Completed in {:.1}s • {} clean • {} with secrets",
                duration_secs, self.clean_repos, self.repos_with_secrets)
        }
    }

    fn generate_detailed_summary(&self) -> String {
        let mut lines = Vec::new();

        // Repos with secrets get priority
        if !self.secret_repos.is_empty() {
            lines.push(format!("🔴 REPOS WITH SECRETS ({})", self.secret_repos.len()));
            for (i, (repo_name, repo_path, total, verified)) in self.secret_repos.iter().enumerate() {
                let tree_char = if i == self.secret_repos.len() - 1 { "└─" } else { "├─" };
                let short_path = shorten_path(repo_path, 30);
                let verified_text = if *verified > 0 { format!(" ({} verified)", verified) } else { String::new() };
                lines.push(format!("   {} {:20} {:30} # {} secrets{}", tree_char, repo_name, short_path, total, verified_text));
            }
            lines.push(String::new()); // Add blank line
        }

        // Failed repos
        if !self.failed_repos.is_empty() {
            lines.push(format!("🟠 FAILED SCANS ({})", self.failed_repos.len()));
            for (i, (repo_name, repo_path, error)) in self.failed_repos.iter().enumerate() {
                let tree_char = if i == self.failed_repos.len() - 1 { "└─" } else { "├─" };
                let short_path = shorten_path(repo_path, 30);
                lines.push(format!("   {} {:20} {:30} # {}", tree_char, repo_name, short_path, error));
            }
        }

        // Remove trailing blank line if it exists
        if lines.last() == Some(&String::new()) {
            lines.pop();
        }

        lines.join("\n")
    }
}

/// Sets the terminal title to the specified text
fn set_terminal_title(title: &str) {
    print!("\x1b]0;{}\x07", title);
}

/// Sets the terminal title and ensures it's flushed to the terminal
fn set_terminal_title_and_flush(title: &str) {
    set_terminal_title(title);
    std::io::stdout().flush().unwrap();
}

/// Checks if TruffleHog is installed and available in PATH
async fn check_trufflehog_installed() -> bool {
    match Command::new("trufflehog").arg("--version").output().await {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// Gets platform-specific information for TruffleHog binary download
fn get_platform_info() -> Result<(String, String)> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let (platform, extension) = match (os, arch) {
        ("linux", "x86_64") => ("linux_amd64", "tar.gz"),
        ("linux", "aarch64") => ("linux_arm64", "tar.gz"),
        ("macos", "x86_64") => ("darwin_amd64", "tar.gz"),
        ("macos", "aarch64") => ("darwin_arm64", "tar.gz"),
        ("windows", "x86_64") => ("windows_amd64", "zip"),
        _ => return Err(anyhow::anyhow!("Unsupported platform: {} {}", os, arch)),
    };

    Ok((platform.to_string(), extension.to_string()))
}

/// Prompts user for TruffleHog installation permission
async fn prompt_trufflehog_install() -> Result<bool> {
    println!("🔍 TruffleHog not found. This command requires TruffleHog to scan for secrets.");
    println!();
    print!("Would you like to install TruffleHog {}? [Y/n]: ", TRUFFLEHOG_VERSION);
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let response = input.trim().to_lowercase();
    Ok(response.is_empty() || response.starts_with('y'))
}

/// Downloads and installs TruffleHog binary
async fn install_trufflehog() -> Result<PathBuf> {
    let (platform, extension) = get_platform_info()?;
    let filename = format!("trufflehog_{}_{}.{}", TRUFFLEHOG_VERSION, platform, extension);
    let download_url = format!("{}/{}/{}", TRUFFLEHOG_BASE_URL, TRUFFLEHOG_VERSION, filename);

    // Create install directory
    let install_dir = dirs::home_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not find home directory"))?
        .join(".local")
        .join("bin");

    fs::create_dir_all(&install_dir)?;

    print!("⬇️  Downloading TruffleHog {} for {}...", TRUFFLEHOG_VERSION, platform);
    io::stdout().flush()?;

    // Download the file
    let response = reqwest::get(&download_url).await?;
    if !response.status().is_success() {
        return Err(anyhow::anyhow!("Failed to download TruffleHog: HTTP {}", response.status()));
    }

    let content = response.bytes().await?;

    // Extract and install based on file type
    let binary_path = install_dir.join("trufflehog");

    if extension == "tar.gz" {
        // Extract tar.gz (Linux/macOS)
        let tar = flate2::read::GzDecoder::new(&content[..]);
        let mut archive = tar::Archive::new(tar);

        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?;

            if path.file_name().unwrap_or_default() == "trufflehog" {
                entry.unpack(&binary_path)?;
                break;
            }
        }
    } else if extension == "zip" {
        // Extract zip (Windows)
        let cursor = std::io::Cursor::new(content);
        let mut archive = zip::ZipArchive::new(cursor)?;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            if file.name().ends_with("trufflehog.exe") || file.name().ends_with("trufflehog") {
                let mut outfile = fs::File::create(&binary_path)?;
                std::io::copy(&mut file, &mut outfile)?;
                break;
            }
        }
    }

    // Make executable (Unix-like systems)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = fs::metadata(&binary_path)?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary_path, permissions)?;
    }

    println!("\r✅ TruffleHog installed successfully to {}", binary_path.display());

    Ok(binary_path)
}

/// Runs TruffleHog on a repository with timeout
async fn run_trufflehog(repo_path: &Path, verify: bool, json: bool) -> Result<(bool, String, String)> {
    let timeout_duration = Duration::from_secs(TRUFFLE_TIMEOUT_SECS);

    let mut args = vec!["git", "file://", repo_path.to_str().unwrap()];
    if verify {
        args.push("--verify");
    }
    if json {
        args.push("--json");
    }

    let result = tokio::time::timeout(
        timeout_duration,
        Command::new("trufflehog")
            .args(&args)
            .output()
    ).await;

    match result {
        Ok(Ok(output)) => Ok((
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        )),
        Ok(Err(e)) => Err(e.into()),
        Err(_) => Err(anyhow::anyhow!(
            "TruffleHog scan timed out after {} seconds",
            TRUFFLE_TIMEOUT_SECS
        )),
    }
}

/// Parses TruffleHog JSON output to extract findings
fn parse_trufflehog_output(output: &str) -> Result<Vec<TruffleHogFinding>> {
    if output.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut findings = Vec::new();
    for line in output.lines() {
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<TruffleHogFinding>(line) {
            Ok(finding) => findings.push(finding),
            Err(_) => continue, // Skip malformed lines
        }
    }

    Ok(findings)
}

/// Scans a repository with TruffleHog and returns results
async fn check_repo_truffle(repo_path: &Path, verify: bool, json: bool) -> (TruffleStatus, String, u32, u32) {
    match run_trufflehog(repo_path, verify, json).await {
        Ok((success, stdout, stderr)) => {
            if !success && !stderr.is_empty() {
                let error_msg = if stderr.len() > 50 {
                    format!("{}...", &stderr[..47])
                } else {
                    stderr
                };
                return (TruffleStatus::Error, error_msg, 0, 0);
            }

            if json {
                match parse_trufflehog_output(&stdout) {
                    Ok(findings) => {
                        if findings.is_empty() {
                            (TruffleStatus::Clean, "no secrets found".to_string(), 0, 0)
                        } else {
                            let total_secrets = findings.len() as u32;
                            let verified_secrets = findings.iter().filter(|f| f.verified).count() as u32;
                            let message = if verified_secrets > 0 {
                                format!("{} secrets ({} verified)", total_secrets, verified_secrets)
                            } else {
                                format!("{} secrets", total_secrets)
                            };
                            (TruffleStatus::Secrets, message, total_secrets, verified_secrets)
                        }
                    }
                    Err(e) => (TruffleStatus::Error, format!("parse error: {}", e), 0, 0)
                }
            } else {
                // Non-JSON output - simple check
                if stdout.trim().is_empty() {
                    (TruffleStatus::Clean, "no secrets found".to_string(), 0, 0)
                } else {
                    // Count lines that look like findings
                    let secret_count = stdout.lines().filter(|line| !line.trim().is_empty()).count() as u32;
                    (TruffleStatus::Secrets, format!("{} potential secrets", secret_count), secret_count, 0)
                }
            }
        }
        Err(e) => {
            let error_msg = format!("{}", e);
            let cleaned_error = if error_msg.len() > 50 {
                format!("{}...", &error_msg[..47])
            } else {
                error_msg
            };
            (TruffleStatus::Error, cleaned_error, 0, 0)
        }
    }
}

/// Helper function to safely acquire a semaphore permit
async fn acquire_semaphore_permit(
    semaphore: &tokio::sync::Semaphore,
) -> tokio::sync::SemaphorePermit<'_> {
    semaphore
        .acquire()
        .await
        .expect("Failed to acquire semaphore permit for concurrent TruffleHog operations")
}

/// Processes all repositories concurrently for TruffleHog scanning
async fn process_truffle_repositories(
    repositories: Vec<(String, PathBuf)>,
    max_name_length: usize,
    multi_progress: MultiProgress,
    progress_style: ProgressStyle,
    statistics: Arc<Mutex<TruffleStatistics>>,
    semaphore: Arc<tokio::sync::Semaphore>,
    total_repos: usize,
    start_time: std::time::Instant,
    verify: bool,
    json: bool,
) {
    use futures::stream::{FuturesUnordered, StreamExt};

    let mut futures = FuturesUnordered::new();

    // Create all repository progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in &repositories {
        let progress_bar = create_progress_bar(&multi_progress, &progress_style, repo_name);
        progress_bar.set_message(TRUFFLE_SCANNING_MESSAGE);
        repo_progress_bars.push(progress_bar);
    }

    // Add a blank line before the footer
    let separator_pb = multi_progress.add(ProgressBar::new(0));
    separator_pb.set_style(ProgressStyle::default_bar().template(" ").unwrap());
    separator_pb.finish();

    // Create the footer progress bar
    let footer_pb = multi_progress.add(ProgressBar::new(0));
    let footer_style = ProgressStyle::default_bar()
        .template("{wide_msg}")
        .expect("Failed to create footer progress style");
    footer_pb.set_style(footer_style);

    // Initial footer display
    let initial_stats = TruffleStatistics::new();
    let initial_summary = initial_stats.generate_summary(total_repos, start_time.elapsed());
    footer_pb.set_message(initial_summary);

    // Add another blank line after the footer
    let separator_pb2 = multi_progress.add(ProgressBar::new(0));
    separator_pb2.set_style(ProgressStyle::default_bar().template(" ").unwrap());
    separator_pb2.finish();

    for ((repo_name, repo_path), progress_bar) in repositories.into_iter().zip(repo_progress_bars) {
        let stats_clone = Arc::clone(&statistics);
        let semaphore_clone = Arc::clone(&semaphore);
        let footer_clone = footer_pb.clone();

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore_clone).await;

            let (status, message, secrets, verified) = check_repo_truffle(&repo_path, verify, json).await;

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                repo_name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<10}   {}", status.text(), message));
            progress_bar.finish();

            // Update statistics
            let mut stats_guard = acquire_stats_lock(&stats_clone);
            let repo_path_str = repo_path.to_string_lossy();
            stats_guard.update(&repo_name, &repo_path_str, &status, &message, secrets, verified);

            // Update the footer summary after each repository completes
            let current_stats = stats_guard.clone();
            drop(stats_guard);

            let duration = start_time.elapsed();
            let summary = current_stats.generate_summary(total_repos, duration);
            footer_clone.set_message(summary);
        };

        futures.push(future);
    }

    // Wait for all repository operations to complete
    while futures.next().await.is_some() {}

    // Finish the footer progress bar
    footer_pb.finish();

    // Print the final detailed summary if there are any issues to report
    let final_stats = acquire_stats_lock(&statistics);
    let detailed_summary = final_stats.generate_detailed_summary();
    if !detailed_summary.is_empty() {
        println!("\n{}", "━".repeat(70));
        println!("{}", detailed_summary);
        println!("{}", "━".repeat(70));
    }

    // Add final spacing
    println!();
}

/// Main handler for the TruffleHog command
pub async fn handle_truffle_command(auto_install: bool, verify: bool, json: bool) -> Result<()> {
    set_terminal_title("🚀 sync-repos truffle");

    // Check if TruffleHog is installed
    if !check_trufflehog_installed().await {
        if auto_install {
            println!("🔍 TruffleHog not found. Installing automatically...");
            install_trufflehog().await?;
        } else {
            if !prompt_trufflehog_install().await? {
                println!("❌ TruffleHog installation cancelled");
                set_terminal_title_and_flush("✅ sync-repos");
                return Ok(());
            }
            install_trufflehog().await?;
        }
        println!();
    }

    println!();
    print!("🔍 Scanning for git repositories...");
    std::io::stdout().flush().unwrap();

    let start_time = std::time::Instant::now();
    let repos = find_repos();
    if repos.is_empty() {
        println!("\r{}", NO_REPOS_MESSAGE);
        set_terminal_title_and_flush("✅ sync-repos");
        return Ok(());
    }

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 { "repository" } else { "repositories" };
    let scan_mode = if verify { " (with verification)" } else { "" };
    print!("\r🔍 Scanning {} {} for secrets{}                    \n", total_repos, repo_word, scan_mode);
    println!();

    // Setup for concurrent processing
    let max_name_length = repos.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
    let multi_progress = MultiProgress::new();
    let progress_style = match create_progress_style() {
        Ok(style) => style,
        Err(e) => {
            set_terminal_title_and_flush("✅ sync-repos");
            return Err(e);
        }
    };
    let statistics = Arc::new(Mutex::new(TruffleStatistics::new()));
    let semaphore = Arc::new(tokio::sync::Semaphore::new(DEFAULT_CONCURRENT_LIMIT));

    // Process all repositories concurrently
    process_truffle_repositories(
        repos,
        max_name_length,
        multi_progress,
        progress_style,
        statistics,
        semaphore,
        total_repos,
        start_time,
        verify,
        json,
    )
    .await;

    set_terminal_title_and_flush("✅ sync-repos");
    Ok(())
}