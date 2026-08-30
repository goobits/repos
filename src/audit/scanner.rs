//! `TruffleHog` integration and secret scanning functionality
//!
//! This module orchestrates the complete audit process including:
//! - `TruffleHog` installation and verification
//! - Secret scanning across repositories
//! - Repository hygiene checking integration
//! - Statistical reporting and progress tracking

mod installer;
mod report;

use anyhow::{anyhow, Result};
use serde_json;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

use installer::{ensure_trufflehog_installed, is_trufflehog_installed};
pub use report::{ReportedSecretFinding, TruffleStatistics};

use super::hygiene::{process_hygiene_repositories, HygieneStatistics};
use crate::core::{
    create_generic_processing_context, init_command, init_command_quiet,
    set_terminal_title_and_flush, GenericProcessingContext, HYGIENE_CONCURRENT_LIMIT,
    NO_REPOS_MESSAGE, TRUFFLE_CONCURRENT_LIMIT,
};

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";

/// Individual secret finding from `TruffleHog`
#[derive(Debug, Clone)]
pub struct SecretFinding {
    pub detector_name: String,
    pub verified: bool,
    pub file_path: String,
}

/// Runs complete `TruffleHog` secret scanning and hygiene checking
/// Returns (`truffle_stats`, `hygiene_stats`)
pub async fn run_truffle_scan(
    install_tools: bool,
    verify: bool,
    json: bool,
    target_repos: Option<Vec<String>>,
) -> Result<(TruffleStatistics, HygieneStatistics)> {
    let (start_time, repos) = if json {
        init_command_quiet().await?
    } else {
        init_command(SCANNING_MESSAGE).await?
    };

    if repos.is_empty() {
        if !json {
            println!("\r{NO_REPOS_MESSAGE}");
        }
        if !json {
            set_terminal_title_and_flush("✅ repos");
        }
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
        if !json {
            println!("\r❌ No matching repositories found");
        }
        if !json {
            set_terminal_title_and_flush("✅ repos");
        }
        return Ok((TruffleStatistics::new(), HygieneStatistics::new()));
    }

    let total_repos = repos_to_scan.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    if !json {
        print!("\r🔍 Auditing {total_repos} {repo_word}                    \n");
        println!();
    }

    // Install TruffleHog if requested and not already installed
    if install_tools {
        ensure_trufflehog_installed().await?;
    }

    // Check if TruffleHog is available
    if !is_trufflehog_installed().await {
        return Err(anyhow!(
            "TruffleHog is not installed. Please install it or use --install-tools:\n\
             brew install trufflesecurity/trufflehog/trufflehog (macOS)\n\
             Install a trusted TruffleHog release for your platform (Linux)\n\
             Or use: repos audit --install-tools"
        ));
    }

    // Wrap repositories in Arc to avoid cloning
    let repos_arc = Arc::new(repos_to_scan);

    // Create processing context for TruffleHog scanning
    let truffle_context = create_generic_processing_context(
        Arc::clone(&repos_arc),
        start_time,
        TruffleStatistics::new(),
        TRUFFLE_CONCURRENT_LIMIT,
    )?;

    // Run TruffleHog scanning
    let truffle_stats = run_truffle_scanning(truffle_context, verify).await?;

    // Create processing context for hygiene checking
    let hygiene_context = create_generic_processing_context(
        Arc::clone(&repos_arc),
        start_time,
        HygieneStatistics::new(),
        HYGIENE_CONCURRENT_LIMIT,
    )?;

    // Run hygiene checking
    let hygiene_stats = process_hygiene_repositories(hygiene_context, !json).await;

    // Get final statistics
    let final_truffle_stats = {
        let mut stats = truffle_stats;
        stats.scan_duration = start_time.elapsed();
        stats
    };

    // JSON is rendered by the command handler after optional fixes so stdout
    // contains exactly one complete document.
    if !json {
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

    Ok((final_truffle_stats, hygiene_stats))
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

    let mut child = Command::new("trufflehog")
        .args(&args)
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("TruffleHog stdout was unavailable"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("TruffleHog stderr was unavailable"))?;
    let stderr_task = tokio::spawn(async move {
        let mut output = Vec::new();
        stderr.read_to_end(&mut output).await.map(|_| output)
    });

    let mut findings = Vec::new();
    let mut lines = BufReader::new(stdout).split(b'\n');
    while let Some(line) = lines.next_segment().await? {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match parse_truffle_finding(&line) {
            Ok(finding) => findings.push(finding),
            Err(error) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let _ = stderr_task.await;
                return Err(error);
            }
        }
    }

    let status = child.wait().await?;
    let stderr = stderr_task.await??;
    if !status.success() {
        return Err(anyhow!(
            "TruffleHog failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        ));
    }

    Ok(findings)
}

fn parse_truffle_finding(line: &[u8]) -> Result<SecretFinding> {
    let json = serde_json::from_slice::<serde_json::Value>(line)
        .map_err(|error| anyhow!("invalid TruffleHog JSON output: {error}"))?;
    let detector_name = json["DetectorName"]
        .as_str()
        .unwrap_or("Unknown")
        .to_string();
    let verified = json["Verified"].as_bool().unwrap_or(false);
    let file_path = json["SourceMetadata"]["Data"]["Git"]["file"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    Ok(SecretFinding {
        detector_name,
        verified,
        file_path,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_truffle_finding;

    #[test]
    fn rejects_malformed_trufflehog_output() {
        let result = parse_truffle_finding(b"not-json");
        assert!(result.is_err());
    }
}
