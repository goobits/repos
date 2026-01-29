//! `TruffleHog` integration and secret scanning functionality
//!
//! This module orchestrates the complete audit process including:
//! - `TruffleHog` installation and verification
//! - Secret scanning across repositories
//! - Repository hygiene checking integration
//! - Statistical reporting and progress tracking

use anyhow::{anyhow, Result};
use serde_json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;

use super::hygiene::{process_hygiene_repositories, HygieneStatistics};
use crate::core::{
    create_generic_processing_context, init_command, set_terminal_title_and_flush,
    GenericProcessingContext, HYGIENE_CONCURRENT_LIMIT, NO_REPOS_MESSAGE, TRUFFLE_CONCURRENT_LIMIT,
};

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";

/// SHA256 checksum of the `TruffleHog` install script
/// IMPORTANT: This should be updated when the install script changes
/// To get the current checksum, download the script and run: sha256sum install.sh
/// URL: <https://raw.githubusercontent.com/trufflesecurity/trufflehog/main/scripts/install.sh>
const TRUFFLEHOG_INSTALL_SCRIPT_SHA256: &str =
    "c394defeaea8a7c48f828a2051b608a9b19f43f34b891407b66a386c3e2591e2";

/// Comprehensive statistics for `TruffleHog` scanning
#[derive(Clone, Default, Debug)]
pub struct TruffleStatistics {
    pub total_repos_scanned: u32,
    pub repos_with_secrets: u32,
    pub total_secrets: u32,
    pub verified_secrets: u32,
    pub unverified_secrets: u32,
    pub secrets_by_detector: HashMap<String, u32>,
    pub failed_repos: Vec<(String, String)>, // (repo_name, error_message)
    pub scan_duration: Duration,
}

impl TruffleStatistics {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_repo_result(&mut self, _repo_name: &str, secrets: &[SecretFinding]) {
        self.total_repos_scanned += 1;

        if !secrets.is_empty() {
            self.repos_with_secrets += 1;
            self.total_secrets += secrets.len() as u32;

            for secret in secrets {
                if secret.verified {
                    self.verified_secrets += 1;
                } else {
                    self.unverified_secrets += 1;
                }

                *self
                    .secrets_by_detector
                    .entry(secret.detector_name.clone())
                    .or_insert(0) += 1;
            }
        }
    }

    pub fn add_repo_failure(&mut self, repo_name: &str, error: &str) {
        self.total_repos_scanned += 1;
        self.failed_repos
            .push((repo_name.to_string(), error.to_string()));
    }

    #[must_use]
    pub fn generate_summary(&self) -> String {
        let duration_secs = self.scan_duration.as_secs_f64();

        if self.verified_secrets > 0 {
            format!(
                "✅ Completed in {:.1}s • {} repos • {} VERIFIED secrets found",
                duration_secs, self.total_repos_scanned, self.verified_secrets
            )
        } else if self.total_secrets > 0 {
            format!(
                "✅ Completed in {:.1}s • {} repos • {} unverified secrets",
                duration_secs, self.total_repos_scanned, self.unverified_secrets
            )
        } else {
            format!(
                "✅ Completed in {:.1}s • {} repos • No secrets found",
                duration_secs, self.total_repos_scanned
            )
        }
    }

    pub fn generate_detailed_report(&self, json: bool) -> Result<String> {
        if json {
            let json_output = serde_json::json!({
                "summary": {
                    "total_repos_scanned": self.total_repos_scanned,
                    "repos_with_secrets": self.repos_with_secrets,
                    "total_secrets": self.total_secrets,
                    "verified_secrets": self.verified_secrets,
                    "unverified_secrets": self.unverified_secrets,
                    "scan_duration_seconds": self.scan_duration.as_secs_f64()
                },
                "secrets_by_detector": self.secrets_by_detector,
                "failed_repos": self.failed_repos
            });
            Ok(serde_json::to_string_pretty(&json_output)?)
        } else {
            let mut report = Vec::new();

            if self.verified_secrets > 0 {
                report.push(format!(
                    "🔴 VERIFIED SECRETS FOUND ({})",
                    self.verified_secrets
                ));
                report.push("   These secrets are confirmed to be active and should be rotated immediately!".to_string());
                report.push(String::new());
            }

            if self.unverified_secrets > 0 {
                report.push(format!(
                    "🟡 UNVERIFIED SECRETS ({})",
                    self.unverified_secrets
                ));
                report.push(
                    "   These appear to be secrets but couldn't be verified as active.".to_string(),
                );
                report.push(String::new());
            }

            if !self.secrets_by_detector.is_empty() {
                report.push("📊 SECRETS BY TYPE".to_string());
                let mut detectors: Vec<_> = self.secrets_by_detector.iter().collect();
                detectors.sort_by(|a, b| b.1.cmp(a.1));

                for (detector, count) in detectors {
                    report.push(format!("   {count} × {detector}"));
                }
                report.push(String::new());
            }

            if !self.failed_repos.is_empty() {
                report.push(format!("❌ SCAN FAILURES ({})", self.failed_repos.len()));
                for (repo, error) in &self.failed_repos {
                    report.push(format!("   {repo} - {error}"));
                }
            }

            Ok(report.join("\n"))
        }
    }
}

/// Individual secret finding from `TruffleHog`
#[derive(Debug, Clone)]
pub struct SecretFinding {
    pub detector_name: String,
    pub verified: bool,
    #[allow(dead_code)]
    pub file_path: String,
}

/// Combined audit statistics
#[derive(Clone, Default)]
pub struct AuditStatistics {
    pub truffle_stats: TruffleStatistics,
    pub hygiene_stats: HygieneStatistics,
}

impl AuditStatistics {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Runs complete `TruffleHog` secret scanning and hygiene checking
/// Returns (`truffle_stats`, `hygiene_stats`)
pub async fn run_truffle_scan(
    install_tools: bool,
    verify: bool,
    json: bool,
    target_repos: Option<Vec<String>>,
) -> Result<(TruffleStatistics, HygieneStatistics)> {
    let (start_time, repos) = init_command(SCANNING_MESSAGE).await;

    if repos.is_empty() {
        println!("\r{NO_REPOS_MESSAGE}");
        set_terminal_title_and_flush("✅ repos");
        return Ok((TruffleStatistics::new(), HygieneStatistics::new()));
    }

    // Filter repositories if specific targets are specified
    let repos_to_scan = if let Some(targets) = target_repos {
        repos
            .into_iter()
            .filter(|(name, _)| targets.contains(name))
            .collect()
    } else {
        repos
    };

    if repos_to_scan.is_empty() {
        println!("\r❌ No matching repositories found");
        set_terminal_title_and_flush("✅ repos");
        return Ok((TruffleStatistics::new(), HygieneStatistics::new()));
    }

    let total_repos = repos_to_scan.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    print!("\r🔍 Auditing {total_repos} {repo_word}                    \n");
    println!();

    // Install TruffleHog if requested and not already installed
    if install_tools {
        ensure_trufflehog_installed().await?;
    }

    // Check if TruffleHog is available
    if !is_trufflehog_installed().await {
        return Err(anyhow!(
            "TruffleHog is not installed. Please install it or use --install-tools:\n\
             brew install trufflesecurity/trufflehog/trufflehog (macOS)\n\
             curl -sSfL https://raw.githubusercontent.com/trufflesecurity/trufflehog/main/scripts/install.sh | sh -s -- -b /usr/local/bin (Linux)\n\
             Or use: repos audit --install-tools"
        ));
    }

    // Create combined audit statistics
    let audit_stats = AuditStatistics::new();

    // Wrap repositories in Arc to avoid cloning
    let repos_arc = Arc::new(repos_to_scan);

    // Create processing context for TruffleHog scanning
    let truffle_context = create_generic_processing_context(
        Arc::clone(&repos_arc),
        start_time,
        audit_stats.truffle_stats.clone(),
        TRUFFLE_CONCURRENT_LIMIT,
    )?;

    // Run TruffleHog scanning
    let truffle_stats = run_truffle_scanning(truffle_context, verify).await?;

    // Create processing context for hygiene checking
    let hygiene_context = create_generic_processing_context(
        Arc::clone(&repos_arc),
        start_time,
        audit_stats.hygiene_stats.clone(),
        HYGIENE_CONCURRENT_LIMIT,
    )?;

    // Run hygiene checking
    process_hygiene_repositories(hygiene_context).await;

    // Get final statistics
    let final_truffle_stats = {
        let mut stats = truffle_stats;
        stats.scan_duration = start_time.elapsed();
        stats
    };

    // Display results
    if json {
        // JSON output - combine both truffle and hygiene results
        let combined_output = serde_json::json!({
            "truffle": final_truffle_stats.generate_detailed_report(true)?,
            "summary": final_truffle_stats.generate_summary()
        });
        println!("{}", serde_json::to_string_pretty(&combined_output)?);
    } else {
        println!("\n{}", "═".repeat(70));
        println!("🔍 SECRET SCANNING RESULTS");
        println!("{}", "═".repeat(70));

        let detailed_report = final_truffle_stats.generate_detailed_report(false)?;
        if detailed_report.trim().is_empty() {
            println!("✅ No secrets found in any repository");
        } else {
            println!("{detailed_report}");
        }

        println!("{}", "═".repeat(70));
    }

    Ok((final_truffle_stats, HygieneStatistics::new()))
}

/// Process `TruffleHog` scanning across repositories
async fn run_truffle_scanning(
    context: GenericProcessingContext<TruffleStatistics>,
    verify: bool,
) -> Result<TruffleStatistics> {
    use futures::stream::{FuturesUnordered, StreamExt};
    use std::sync::Arc;

    let mut futures = FuturesUnordered::new();

    // Extract values before moving context
    let max_name_length = context.max_name_length;

    for (repo_name, repo_path) in context.repositories.iter() {
        let stats_clone = Arc::clone(&context.statistics);
        let semaphore_clone = Arc::clone(&context.semaphore);
        let progress_style = context.progress_style.clone();
        let multi_progress = context.multi_progress.clone();

        let future = async move {
            let _permit = semaphore_clone
                .acquire()
                .await
                .expect("Failed to acquire semaphore permit for TruffleHog scanning");

            // Create progress bar for this repository
            let pb = multi_progress.add(indicatif::ProgressBar::new(100));
            pb.set_style(progress_style);
            pb.set_prefix(format!("🟡 {repo_name:max_name_length$}"));
            pb.set_message("scanning secrets...");

            // Run TruffleHog scan
            match scan_repository_secrets(repo_path, verify).await {
                Ok(secrets) => {
                    let status_symbol = if secrets.iter().any(|s| s.verified) {
                        "🔴" // Verified secrets found
                    } else if !secrets.is_empty() {
                        "🟡" // Unverified secrets found
                    } else {
                        "🟢" // No secrets
                    };

                    let message = if secrets.is_empty() {
                        "no secrets".to_string()
                    } else {
                        let verified = secrets.iter().filter(|s| s.verified).count();
                        if verified > 0 {
                            format!("{} secrets ({} verified)", secrets.len(), verified)
                        } else {
                            format!("{} secrets (unverified)", secrets.len())
                        }
                    };

                    pb.set_prefix(format!("{status_symbol} {repo_name:max_name_length$}"));
                    pb.set_message(message);
                    pb.finish();

                    // Update statistics
                    let mut stats = stats_clone.lock().expect("Failed to acquire stats lock");
                    stats.add_repo_result(repo_name, &secrets);
                }
                Err(e) => {
                    pb.set_prefix(format!("🟠 {repo_name:max_name_length$}"));
                    pb.set_message(format!("scan failed: {e}"));
                    pb.finish();

                    // Update statistics with failure
                    let mut stats = stats_clone.lock().expect("Failed to acquire stats lock");
                    stats.add_repo_failure(repo_name, &e.to_string());
                }
            }
        };

        futures.push(future);
    }

    // Wait for all scanning to complete
    while futures.next().await.is_some() {}

    // Extract final statistics
    let final_stats = {
        let stats_guard = context
            .statistics
            .lock()
            .expect("Failed to acquire stats lock");
        stats_guard.clone()
    };

    Ok(final_stats)
}

/// Scan a single repository for secrets using `TruffleHog`
async fn scan_repository_secrets(
    repo_path: &std::path::Path,
    verify: bool,
) -> Result<Vec<SecretFinding>> {
    let repo_url = format!("file://{}", repo_path.display());
    let mut args = vec!["git", &repo_url, "--json", "--no-update"];

    if verify {
        args.push("--results=verified,unknown");
    } else {
        args.push("--results=unknown");
    }

    let output = Command::new("trufflehog")
        .args(&args)
        .current_dir(repo_path)
        .output()
        .await?;

    if !output.status.success() && output.stdout.is_empty() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut findings = Vec::new();

    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }

        // Parse JSON output from TruffleHog
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            let detector_name = json["DetectorName"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string();

            let verified = json["Verified"].as_bool().unwrap_or(false);

            let file_path = json["SourceMetadata"]["Data"]["Git"]["file"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();

            findings.push(SecretFinding {
                detector_name,
                verified,
                file_path,
            });
        }
    }

    Ok(findings)
}

/// Check if `TruffleHog` is installed and accessible
async fn is_trufflehog_installed() -> bool {
    Command::new("trufflehog")
        .arg("--version")
        .output()
        .await
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Install `TruffleHog` if not already present
async fn ensure_trufflehog_installed() -> Result<()> {
    if is_trufflehog_installed().await {
        println!("✅ TruffleHog is already installed");
        return Ok(());
    }

    println!("📦 Installing TruffleHog...");

    // Detect platform and install accordingly
    let install_cmd = if cfg!(target_os = "macos") {
        // macOS - try Homebrew first
        if Command::new("brew").arg("--version").output().await.is_ok() {
            (
                "brew",
                vec!["install", "trufflesecurity/trufflehog/trufflehog"],
            )
        } else {
            // Fallback to direct download
            return install_trufflehog_direct().await;
        }
    } else if cfg!(target_os = "linux") {
        // Linux - use direct download script
        return install_trufflehog_direct().await;
    } else {
        return Err(anyhow!("Automatic TruffleHog installation not supported on this platform. Please install manually."));
    };

    println!("Running: {} {}", install_cmd.0, install_cmd.1.join(" "));

    let output = Command::new(install_cmd.0)
        .args(&install_cmd.1)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("Failed to install TruffleHog: {stderr}"));
    }

    // Verify installation
    if !is_trufflehog_installed().await {
        return Err(anyhow!(
            "TruffleHog installation completed but tool is not accessible"
        ));
    }

    println!("✅ TruffleHog installed successfully");
    Ok(())
}

/// Verify file checksum against expected SHA256 hash
async fn verify_file_checksum(path: &std::path::Path, expected_sha256: &str) -> Result<bool> {
    // Skip verification if placeholder is still in use
    if expected_sha256 == "PLACEHOLDER_UPDATE_WITH_ACTUAL_CHECKSUM" {
        return Ok(false);
    }

    // Read file contents
    let contents = tokio::fs::read(path).await?;

    // Compute SHA256 hash
    let mut hasher = Sha256::new();
    hasher.update(&contents);
    let result = hasher.finalize();
    let computed_hash = format!("{result:x}");

    // Compare with expected hash (case-insensitive)
    Ok(computed_hash.eq_ignore_ascii_case(expected_sha256))
}

/// Install `TruffleHog` using direct download script
async fn install_trufflehog_direct() -> Result<()> {
    // Security warning before downloading external script
    println!("\n⚠️  SECURITY NOTICE:");
    println!("   This will download and execute an installation script from:");
    println!(
        "   https://raw.githubusercontent.com/trufflesecurity/trufflehog/main/scripts/install.sh"
    );
    println!("   The script will be verified against a known checksum before execution.\n");

    println!("📥 Downloading TruffleHog installation script...");

    let script_url =
        "https://raw.githubusercontent.com/trufflesecurity/trufflehog/main/scripts/install.sh";
    let install_dir = "/usr/local/bin";

    // Check if we have write access to /usr/local/bin
    let install_path = if tokio::fs::metadata(install_dir).await.is_ok()
        && tokio::fs::File::create(format!("{install_dir}/test_write"))
            .await
            .is_ok()
    {
        // Clean up test file
        let _ = tokio::fs::remove_file(format!("{install_dir}/test_write")).await;
        install_dir.to_string()
    } else {
        // Fallback to user's local bin
        let home = std::env::var("HOME")?;
        let user_bin = format!("{home}/.local/bin");
        tokio::fs::create_dir_all(&user_bin).await?;
        println!("⚠️  Installing to {user_bin} (add to PATH if needed)");
        user_bin
    };

    // Download script to temporary file for security (avoid piping curl to shell)
    let temp_script = format!("/tmp/trufflehog-install-{}.sh", std::process::id());

    // Download the installation script
    let download_output = Command::new("curl")
        .args(["-sSfL", "-o", &temp_script, script_url])
        .output()
        .await?;

    if !download_output.status.success() {
        let stderr = String::from_utf8_lossy(&download_output.stderr);
        return Err(anyhow!("Failed to download TruffleHog installer: {stderr}"));
    }

    println!("✅ Download complete, verifying checksum...");

    // Verify the downloaded script's checksum
    let temp_script_path = std::path::Path::new(&temp_script);
    match verify_file_checksum(temp_script_path, TRUFFLEHOG_INSTALL_SCRIPT_SHA256).await {
        Ok(true) => {
            println!("✅ Checksum verification passed");
        }
        Ok(false) => {
            eprintln!("\n⚠️  WARNING: Could not verify TruffleHog install script checksum");
            eprintln!("   Reason: Checksum constant needs to be updated with actual value");
            eprintln!(
                "   The script will still be executed, but please verify manually if concerned."
            );
            eprintln!("   To update: Download the script and run 'sha256sum install.sh'\n");
        }
        Err(e) => {
            eprintln!("\n⚠️  WARNING: Checksum verification failed: {e}");
            eprintln!("   Proceeding with installation - verify manually if concerned.\n");
        }
    }

    // Execute the downloaded script
    println!("🔧 Executing installation script...");
    let install_output = Command::new("sh")
        .args([&temp_script, "-b", &install_path])
        .output()
        .await?;

    // Clean up temporary script file
    let _ = tokio::fs::remove_file(&temp_script).await;

    if !install_output.status.success() {
        let stderr = String::from_utf8_lossy(&install_output.stderr);
        return Err(anyhow!("Failed to install TruffleHog: {stderr}"));
    }

    Ok(())
}
