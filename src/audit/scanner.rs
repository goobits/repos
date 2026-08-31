//! `TruffleHog` integration and secret scanning functionality
//!
//! This module orchestrates the complete audit process including:
//! - `TruffleHog` installation and verification
//! - Secret scanning across repositories
//! - Repository hygiene checking integration
//! - Statistical reporting and progress tracking

mod installer;
mod report;

use anyhow::{anyhow, bail, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;

use installer::{ensure_trufflehog_installed, is_trufflehog_installed};
pub use report::{ReportedSecretFinding, SecretVerification, TruffleStatistics};

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

/// One immutable repository inventory used by both audit scanning and fixes.
#[derive(Clone)]
pub struct AuditScanResult {
    pub repositories: Vec<(String, PathBuf)>,
    pub truffle_statistics: TruffleStatistics,
    pub hygiene_statistics: HygieneStatistics,
}

#[derive(Clone, Copy)]
pub(crate) enum TruffleScanMode {
    Offline,
    Verify,
}

/// Full scanner output retained only long enough to construct a safe rewrite.
///
/// Do not derive `Debug` or `Serialize`: these fields can contain live secrets.
pub(crate) struct ScannedSecret {
    pub(crate) finding: SecretFinding,
    pub(crate) verification: SecretVerification,
    pub(crate) raw: Option<String>,
    pub(crate) raw_v2: Option<String>,
}

/// Runs complete `TruffleHog` secret scanning and hygiene checking
/// Returns the exact repository inventory and both result sets.
pub async fn run_truffle_scan(
    install_tools: bool,
    verify: bool,
    json: bool,
    target_repos: Option<Vec<String>>,
) -> Result<AuditScanResult> {
    let (start_time, repos) = if json {
        init_command_quiet().await?
    } else {
        init_command(SCANNING_MESSAGE).await?
    };

    let repos_to_scan = select_audit_repositories(repos, target_repos.as_deref())?;

    if repos_to_scan.is_empty() {
        if !json {
            println!("\r{NO_REPOS_MESSAGE}");
        }
        if !json {
            set_terminal_title_and_flush("✅ repos");
        }
        return Ok(AuditScanResult {
            repositories: Vec::new(),
            truffle_statistics: TruffleStatistics::new(),
            hygiene_statistics: HygieneStatistics::new(),
        });
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

    Ok(AuditScanResult {
        repositories: repos_arc.as_ref().clone(),
        truffle_statistics: final_truffle_stats,
        hygiene_statistics: hygiene_stats,
    })
}

fn select_audit_repositories(
    repositories: Vec<(String, PathBuf)>,
    targets: Option<&[String]>,
) -> Result<Vec<(String, PathBuf)>> {
    let Some(targets) = targets else {
        return Ok(repositories);
    };

    let requested = targets.iter().cloned().collect::<HashSet<_>>();
    let matched = repositories
        .iter()
        .filter(|(name, _)| requested.contains(name))
        .map(|(name, _)| name.clone())
        .collect::<HashSet<_>>();
    let mut missing = requested.difference(&matched).cloned().collect::<Vec<_>>();
    missing.sort();
    if !missing.is_empty() {
        bail!(
            "no repositories matched requested targets: {}",
            missing.join(", ")
        );
    }

    Ok(repositories
        .into_iter()
        .filter(|(name, _)| requested.contains(name))
        .collect())
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
            let mode = if verify {
                TruffleScanMode::Verify
            } else {
                TruffleScanMode::Offline
            };
            match scan_repository_secrets(repo_path, mode).await {
                Ok(secrets) => {
                    let status_symbol = if secrets
                        .iter()
                        .any(|secret| secret.verification == SecretVerification::Verified)
                    {
                        "🔴" // Verified secrets found
                    } else if !secrets.is_empty() {
                        "🟡" // Unverified secrets found
                    } else {
                        "🟢" // No secrets
                    };

                    let message = if secrets.is_empty() {
                        "no secrets".to_string()
                    } else {
                        let verified = secrets
                            .iter()
                            .filter(|secret| secret.verification == SecretVerification::Verified)
                            .count();
                        let unknown = secrets
                            .iter()
                            .filter(|secret| secret.verification == SecretVerification::Unknown)
                            .count();
                        if verified > 0 {
                            format!("{} secrets ({} verified)", secrets.len(), verified)
                        } else if unknown > 0 {
                            format!(
                                "{} secrets ({} verification unknown)",
                                secrets.len(),
                                unknown
                            )
                        } else {
                            format!("{} secrets (unverified)", secrets.len())
                        }
                    };

                    pb.set_prefix(format!("{status_symbol} {repo_name:max_name_length$}"));
                    pb.set_message(message);
                    pb.finish();

                    // Update statistics
                    let mut stats = stats_clone.lock().expect("Failed to acquire stats lock");
                    stats.add_scanned_repo_result(repo_name, repo_path, &secrets);
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
pub(crate) async fn scan_repository_secrets(
    repo_path: &Path,
    mode: TruffleScanMode,
) -> Result<Vec<ScannedSecret>> {
    let repo_url = format!("file://{}", repo_path.display());
    let args = trufflehog_args(repo_url, mode);

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

fn trufflehog_args(repo_url: String, mode: TruffleScanMode) -> Vec<String> {
    let mut args = vec![
        "git".to_string(),
        repo_url,
        "--json".to_string(),
        "--no-update".to_string(),
        "--fail-on-scan-errors".to_string(),
    ];
    match mode {
        TruffleScanMode::Offline => {
            args.push("--no-verification".to_string());
            args.push("--results=unverified".to_string());
        }
        TruffleScanMode::Verify => {
            args.push("--results=verified,unverified,unknown".to_string());
        }
    }
    args
}

fn parse_truffle_finding(line: &[u8]) -> Result<ScannedSecret> {
    let output = serde_json::from_slice::<TruffleHogOutput>(line)
        .map_err(|error| anyhow!("invalid TruffleHog JSON output: {error}"))?;
    if output.detector_name.trim().is_empty() {
        bail!("invalid TruffleHog JSON output: DetectorName was empty");
    }
    if output.source_metadata.data.git.file.is_empty() {
        bail!("invalid TruffleHog JSON output: Git file was empty");
    }
    let verification = if output.verified {
        SecretVerification::Verified
    } else if output
        .verification_error
        .as_deref()
        .is_some_and(|error| !error.trim().is_empty())
    {
        SecretVerification::Unknown
    } else {
        SecretVerification::Unverified
    };
    Ok(ScannedSecret {
        finding: SecretFinding {
            detector_name: output.detector_name,
            verified: output.verified,
            file_path: output.source_metadata.data.git.file,
        },
        verification,
        raw: nonempty(output.raw),
        raw_v2: nonempty(output.raw_v2),
    })
}

fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TruffleHogOutput {
    detector_name: String,
    verified: bool,
    #[serde(default)]
    verification_error: Option<String>,
    #[serde(default)]
    raw: Option<String>,
    #[serde(default)]
    raw_v2: Option<String>,
    source_metadata: TruffleSourceMetadata,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TruffleSourceMetadata {
    data: TruffleSourceData,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct TruffleSourceData {
    git: TruffleGitMetadata,
}

#[derive(Deserialize)]
struct TruffleGitMetadata {
    file: String,
}

#[cfg(test)]
mod tests {
    use super::{
        parse_truffle_finding, select_audit_repositories, trufflehog_args, SecretVerification,
        TruffleScanMode,
    };
    use std::path::PathBuf;

    #[test]
    fn rejects_malformed_trufflehog_output() {
        let result = parse_truffle_finding(b"not-json");
        assert!(result.is_err());
    }

    #[test]
    fn offline_scan_is_non_verifying_and_fail_closed() {
        let args = trufflehog_args("file:///repo".to_string(), TruffleScanMode::Offline);
        assert!(args.iter().any(|arg| arg == "--no-verification"));
        assert!(args.iter().any(|arg| arg == "--results=unverified"));
        assert!(args.iter().any(|arg| arg == "--fail-on-scan-errors"));
    }

    #[test]
    fn verification_scan_retains_every_result_class() {
        let args = trufflehog_args("file:///repo".to_string(), TruffleScanMode::Verify);
        assert!(!args.iter().any(|arg| arg == "--no-verification"));
        assert!(args
            .iter()
            .any(|arg| arg == "--results=verified,unverified,unknown"));
        assert!(args.iter().any(|arg| arg == "--fail-on-scan-errors"));
    }

    #[test]
    fn parser_classifies_verification_errors_as_unknown() {
        let finding = parse_truffle_finding(
            br#"{"DetectorName":"AWS","Verified":false,"VerificationError":"timeout","Raw":"token","SourceMetadata":{"Data":{"Git":{"file":"secrets.env"}}}}"#,
        )
        .expect("valid finding");
        assert_eq!(finding.verification, SecretVerification::Unknown);
        assert_eq!(finding.raw.as_deref(), Some("token"));
    }

    #[test]
    fn parser_rejects_missing_required_fields() {
        let result = parse_truffle_finding(br#"{"DetectorName":"AWS","Verified":false}"#);
        assert!(result.is_err());
    }

    #[test]
    fn target_selection_reports_every_missing_name() {
        let repos = vec![("present".to_string(), PathBuf::from("/present"))];
        let targets = vec!["missing-b".to_string(), "missing-a".to_string()];
        let error = select_audit_repositories(repos, Some(&targets)).expect_err("missing targets");
        assert!(error.to_string().contains("missing-a, missing-b"));
    }

    #[test]
    fn target_selection_keeps_duplicate_repository_names_once_per_path() {
        let repos = vec![
            ("same".to_string(), PathBuf::from("/one/same")),
            ("same".to_string(), PathBuf::from("/two/same")),
        ];
        let targets = vec!["same".to_string(), "same".to_string()];
        let selected = select_audit_repositories(repos, Some(&targets)).expect("matching targets");
        assert_eq!(selected.len(), 2);
    }
}
