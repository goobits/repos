//! Safe secret-finding models and human/JSON report generation.

use super::{ScannedSecret, SecretFinding};
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretVerification {
    Verified,
    Unknown,
    Unverified,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ReportedSecretFinding {
    pub repository: String,
    pub detector: String,
    pub verified: bool,
    pub verification: SecretVerification,
    pub file: String,
}

#[derive(Clone, Default, Debug)]
pub struct TruffleStatistics {
    pub total_repos_scanned: u32,
    pub repos_with_secrets: u32,
    pub total_secrets: u32,
    pub verified_secrets: u32,
    pub unknown_secrets: u32,
    pub unverified_secrets: u32,
    pub secrets_by_detector: HashMap<String, u32>,
    pub findings: Vec<ReportedSecretFinding>,
    pub failed_repos: Vec<(String, String)>,
    pub scan_duration: Duration,
    secret_repository_paths: HashSet<PathBuf>,
}

impl TruffleStatistics {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_repo_result(&mut self, repo_name: &str, secrets: &[SecretFinding]) {
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

                self.findings.push(ReportedSecretFinding {
                    repository: repo_name.to_string(),
                    detector: secret.detector_name.clone(),
                    verified: secret.verified,
                    verification: if secret.verified {
                        SecretVerification::Verified
                    } else {
                        SecretVerification::Unverified
                    },
                    file: secret.file_path.clone(),
                });
            }
        }
    }

    pub(crate) fn add_scanned_repo_result(
        &mut self,
        repo_name: &str,
        repo_path: &Path,
        secrets: &[ScannedSecret],
    ) {
        self.total_repos_scanned += 1;
        if secrets.is_empty() {
            return;
        }

        self.repos_with_secrets += 1;
        self.total_secrets += secrets.len() as u32;
        self.secret_repository_paths.insert(repo_path.to_path_buf());
        for secret in secrets {
            match secret.verification {
                SecretVerification::Verified => self.verified_secrets += 1,
                SecretVerification::Unknown => self.unknown_secrets += 1,
                SecretVerification::Unverified => self.unverified_secrets += 1,
            }
            *self
                .secrets_by_detector
                .entry(secret.finding.detector_name.clone())
                .or_insert(0) += 1;
            self.findings.push(ReportedSecretFinding {
                repository: repo_name.to_string(),
                detector: secret.finding.detector_name.clone(),
                verified: secret.finding.verified,
                verification: secret.verification,
                file: secret.finding.file_path.clone(),
            });
        }
    }

    #[must_use]
    pub(crate) fn repository_has_secrets(&self, path: &Path) -> bool {
        self.secret_repository_paths.contains(path)
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
        } else if self.unknown_secrets > 0 {
            format!(
                "✅ Completed in {:.1}s • {} repos • {} secrets with UNKNOWN verification",
                duration_secs, self.total_repos_scanned, self.unknown_secrets
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
            Ok(serde_json::to_string_pretty(&self.to_json())?)
        } else {
            let mut report = Vec::new();

            if self.verified_secrets > 0 {
                report.push(format!(
                    "🔴 VERIFIED SECRETS FOUND ({})",
                    self.verified_secrets
                ));
                report.push("   These secrets are confirmed to be active and should be rotated immediately!".to_string());
                append_findings(&mut report, &self.findings, SecretVerification::Verified);
                report.push(String::new());
            }

            if self.unknown_secrets > 0 {
                report.push(format!(
                    "🟠 UNKNOWN SECRET VERIFICATION ({})",
                    self.unknown_secrets
                ));
                report.push(
                    "   Verification failed; treat these findings as unsafe until reviewed."
                        .to_string(),
                );
                append_findings(&mut report, &self.findings, SecretVerification::Unknown);
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
                append_findings(&mut report, &self.findings, SecretVerification::Unverified);
                report.push(String::new());
            }

            if !self.secrets_by_detector.is_empty() {
                report.push("📊 SECRETS BY TYPE".to_string());
                let mut detectors = self.secrets_by_detector.iter().collect::<Vec<_>>();
                detectors.sort_by(|left, right| right.1.cmp(left.1));

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

    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        let mut findings = self.findings.clone();
        findings.sort_by(|left, right| {
            left.verification
                .cmp(&right.verification)
                .then_with(|| left.repository.cmp(&right.repository))
                .then_with(|| left.file.cmp(&right.file))
                .then_with(|| left.detector.cmp(&right.detector))
        });

        let mut failed_repos = self.failed_repos.clone();
        failed_repos.sort();

        serde_json::json!({
            "summary": {
                "total_repos_scanned": self.total_repos_scanned,
                "repos_with_secrets": self.repos_with_secrets,
                "total_secrets": self.total_secrets,
                "verified_secrets": self.verified_secrets,
                "unknown_secrets": self.unknown_secrets,
                "unverified_secrets": self.unverified_secrets,
                "scan_duration_seconds": self.scan_duration.as_secs_f64()
            },
            "findings": findings,
            "secrets_by_detector": self.secrets_by_detector,
            "failed_repos": failed_repos.into_iter().map(|(repository, error)| {
                serde_json::json!({
                    "repository": repository,
                    "error": error,
                })
            }).collect::<Vec<_>>(),
        })
    }
}

fn append_findings(
    report: &mut Vec<String>,
    findings: &[ReportedSecretFinding],
    verification: SecretVerification,
) {
    let mut matching = findings
        .iter()
        .filter(|finding| finding.verification == verification)
        .collect::<Vec<_>>();
    matching.sort_by(|left, right| {
        left.repository
            .cmp(&right.repository)
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.detector.cmp(&right.detector))
    });

    for finding in matching {
        report.push(format!(
            "   • {} · {} · {}",
            finding.repository, finding.detector, finding.file
        ));
    }
}
