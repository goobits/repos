//! Comprehensive integration tests for the audit system
//!
//! Tests cover:
//! - scanner.rs: Secret scanning statistics, findings, and reporting
//! - hygiene.rs: Repository hygiene checking and violations
//! - fixes.rs: Automated fix operations (with safety checks)

use goobits_repos::audit::fixes::FixOptions;
use goobits_repos::audit::hygiene::report::HygieneStatus;
use goobits_repos::audit::hygiene::{HygieneStatistics, HygieneViolation, ViolationType};
use goobits_repos::audit::scanner::{AuditStatistics, SecretFinding, TruffleStatistics};
use std::time::Duration;

mod common;
use common::{is_git_available, TestRepoBuilder};

// =====================================================================================
// scanner.rs tests - TruffleHog secret scanning functionality
// =====================================================================================

#[test]
fn test_truffle_statistics_initialization() {
    let stats = TruffleStatistics::new();

    assert_eq!(
        stats.total_repos_scanned, 0,
        "Should start with 0 scanned repos"
    );
    assert_eq!(
        stats.repos_with_secrets, 0,
        "Should start with 0 repos with secrets"
    );
    assert_eq!(stats.total_secrets, 0, "Should start with 0 total secrets");
    assert_eq!(
        stats.verified_secrets, 0,
        "Should start with 0 verified secrets"
    );
    assert_eq!(
        stats.unverified_secrets, 0,
        "Should start with 0 unverified secrets"
    );
    assert!(
        stats.secrets_by_detector.is_empty(),
        "Should start with empty detector map"
    );
    assert!(
        stats.failed_repos.is_empty(),
        "Should start with no failed repos"
    );
    assert_eq!(
        stats.scan_duration,
        Duration::default(),
        "Should start with zero duration"
    );
}

#[test]
fn test_truffle_statistics_add_repo_result_no_secrets() {
    let mut stats = TruffleStatistics::new();
    let secrets: Vec<SecretFinding> = vec![];

    stats.add_repo_result("test-repo", &secrets);

    assert_eq!(
        stats.total_repos_scanned, 1,
        "Should increment scanned repos count"
    );
    assert_eq!(
        stats.repos_with_secrets, 0,
        "Should not count repos with no secrets"
    );
    assert_eq!(stats.total_secrets, 0, "Should have 0 total secrets");
}

#[test]
fn test_truffle_statistics_add_repo_result_with_unverified_secrets() {
    let mut stats = TruffleStatistics::new();
    let secrets = vec![
        SecretFinding {
            detector_name: "AWS".to_string(),
            verified: false,
            file_path: "config/aws.yml".to_string(),
        },
        SecretFinding {
            detector_name: "GitHub".to_string(),
            verified: false,
            file_path: ".env".to_string(),
        },
    ];

    stats.add_repo_result("test-repo", &secrets);

    assert_eq!(stats.total_repos_scanned, 1, "Should count repo as scanned");
    assert_eq!(
        stats.repos_with_secrets, 1,
        "Should count repo with secrets"
    );
    assert_eq!(stats.total_secrets, 2, "Should count both secrets");
    assert_eq!(stats.verified_secrets, 0, "Should have 0 verified secrets");
    assert_eq!(
        stats.unverified_secrets, 2,
        "Should have 2 unverified secrets"
    );
    assert_eq!(
        *stats.secrets_by_detector.get("AWS").unwrap(),
        1,
        "Should count AWS detector"
    );
    assert_eq!(
        *stats.secrets_by_detector.get("GitHub").unwrap(),
        1,
        "Should count GitHub detector"
    );
}

#[test]
fn test_truffle_statistics_add_repo_result_with_verified_secrets() {
    let mut stats = TruffleStatistics::new();
    let secrets = vec![
        SecretFinding {
            detector_name: "AWS".to_string(),
            verified: true,
            file_path: "config/aws.yml".to_string(),
        },
        SecretFinding {
            detector_name: "AWS".to_string(),
            verified: false,
            file_path: ".env".to_string(),
        },
    ];

    stats.add_repo_result("test-repo", &secrets);

    assert_eq!(stats.verified_secrets, 1, "Should count 1 verified secret");
    assert_eq!(
        stats.unverified_secrets, 1,
        "Should count 1 unverified secret"
    );
    assert_eq!(
        *stats.secrets_by_detector.get("AWS").unwrap(),
        2,
        "Should count both AWS secrets"
    );
}

#[test]
fn test_truffle_statistics_add_repo_failure() {
    let mut stats = TruffleStatistics::new();

    stats.add_repo_failure("failed-repo", "TruffleHog not installed");

    assert_eq!(
        stats.total_repos_scanned, 1,
        "Should count failed repo as scanned"
    );
    assert_eq!(stats.failed_repos.len(), 1, "Should track failed repo");
    assert_eq!(
        stats.failed_repos[0].0, "failed-repo",
        "Should store repo name"
    );
    assert_eq!(
        stats.failed_repos[0].1, "TruffleHog not installed",
        "Should store error message"
    );
}

#[test]
fn test_truffle_statistics_accumulation_across_repos() {
    let mut stats = TruffleStatistics::new();

    // First repo with secrets
    let secrets1 = vec![SecretFinding {
        detector_name: "AWS".to_string(),
        verified: true,
        file_path: "config.yml".to_string(),
    }];
    stats.add_repo_result("repo1", &secrets1);

    // Second repo with different secrets
    let secrets2 = vec![
        SecretFinding {
            detector_name: "GitHub".to_string(),
            verified: false,
            file_path: ".env".to_string(),
        },
        SecretFinding {
            detector_name: "AWS".to_string(),
            verified: false,
            file_path: "creds.json".to_string(),
        },
    ];
    stats.add_repo_result("repo2", &secrets2);

    // Third repo with no secrets
    stats.add_repo_result("repo3", &[]);

    // Fourth repo failure
    stats.add_repo_failure("repo4", "Scan timeout");

    assert_eq!(stats.total_repos_scanned, 4, "Should count all repos");
    assert_eq!(
        stats.repos_with_secrets, 2,
        "Should count repos with secrets"
    );
    assert_eq!(stats.total_secrets, 3, "Should count all secrets");
    assert_eq!(stats.verified_secrets, 1, "Should count verified secrets");
    assert_eq!(
        stats.unverified_secrets, 2,
        "Should count unverified secrets"
    );
    assert_eq!(
        *stats.secrets_by_detector.get("AWS").unwrap(),
        2,
        "Should aggregate AWS secrets"
    );
    assert_eq!(
        *stats.secrets_by_detector.get("GitHub").unwrap(),
        1,
        "Should count GitHub secret"
    );
    assert_eq!(stats.failed_repos.len(), 1, "Should track failed repo");
}

#[test]
fn test_truffle_statistics_generate_summary_no_secrets() {
    let mut stats = TruffleStatistics::new();
    stats.total_repos_scanned = 5;
    stats.scan_duration = Duration::from_secs_f64(12.5);

    let summary = stats.generate_summary();

    assert!(summary.contains("12.5s"), "Should include duration");
    assert!(summary.contains("5 repos"), "Should include repo count");
    assert!(
        summary.contains("No secrets found"),
        "Should indicate no secrets"
    );
}

#[test]
fn test_truffle_statistics_generate_summary_with_verified_secrets() {
    let mut stats = TruffleStatistics::new();
    stats.total_repos_scanned = 10;
    stats.verified_secrets = 3;
    stats.unverified_secrets = 7;
    stats.total_secrets = 10;
    stats.scan_duration = Duration::from_secs_f64(45.2);

    let summary = stats.generate_summary();

    assert!(summary.contains("45.2s"), "Should include duration");
    assert!(summary.contains("10 repos"), "Should include repo count");
    assert!(
        summary.contains("3 VERIFIED secrets"),
        "Should highlight verified secrets"
    );
    assert!(
        !summary.contains("unverified"),
        "Should prioritize verified in summary"
    );
}

#[test]
fn test_truffle_statistics_generate_summary_with_only_unverified() {
    let mut stats = TruffleStatistics::new();
    stats.total_repos_scanned = 8;
    stats.unverified_secrets = 5;
    stats.total_secrets = 5;
    stats.scan_duration = Duration::from_secs_f64(30.0);

    let summary = stats.generate_summary();

    assert!(summary.contains("30.0s"), "Should include duration");
    assert!(summary.contains("8 repos"), "Should include repo count");
    assert!(
        summary.contains("5 unverified secrets"),
        "Should mention unverified secrets"
    );
}

#[test]
fn test_truffle_statistics_generate_detailed_report_text() {
    let mut stats = TruffleStatistics::new();
    stats.verified_secrets = 2;
    stats.unverified_secrets = 3;
    stats.secrets_by_detector.insert("AWS".to_string(), 3);
    stats.secrets_by_detector.insert("GitHub".to_string(), 2);
    stats
        .failed_repos
        .push(("failed-repo".to_string(), "Timeout".to_string()));

    let report = stats
        .generate_detailed_report(false)
        .expect("Report generation should succeed");

    assert!(
        report.contains("VERIFIED SECRETS FOUND (2)"),
        "Should show verified count"
    );
    assert!(
        report.contains("UNVERIFIED SECRETS (3)"),
        "Should show unverified count"
    );
    assert!(
        report.contains("SECRETS BY TYPE"),
        "Should have detector breakdown"
    );
    assert!(report.contains("3 × AWS"), "Should show AWS count");
    assert!(report.contains("2 × GitHub"), "Should show GitHub count");
    assert!(report.contains("SCAN FAILURES (1)"), "Should show failures");
    assert!(
        report.contains("failed-repo - Timeout"),
        "Should show failure details"
    );
}

#[test]
fn test_truffle_statistics_generate_detailed_report_json() {
    let mut stats = TruffleStatistics::new();
    stats.total_repos_scanned = 5;
    stats.repos_with_secrets = 2;
    stats.total_secrets = 4;
    stats.verified_secrets = 1;
    stats.unverified_secrets = 3;
    stats.scan_duration = Duration::from_secs_f64(20.5);
    stats.secrets_by_detector.insert("AWS".to_string(), 2);
    stats
        .failed_repos
        .push(("repo1".to_string(), "Error".to_string()));

    let json_report = stats
        .generate_detailed_report(true)
        .expect("JSON generation should succeed");

    // Parse JSON to verify structure
    let parsed: serde_json::Value =
        serde_json::from_str(&json_report).expect("Should generate valid JSON");

    assert_eq!(
        parsed["summary"]["total_repos_scanned"], 5,
        "JSON should contain total_repos_scanned"
    );
    assert_eq!(
        parsed["summary"]["repos_with_secrets"], 2,
        "JSON should contain repos_with_secrets"
    );
    assert_eq!(
        parsed["summary"]["total_secrets"], 4,
        "JSON should contain total_secrets"
    );
    assert_eq!(
        parsed["summary"]["verified_secrets"], 1,
        "JSON should contain verified_secrets"
    );
    assert_eq!(
        parsed["summary"]["unverified_secrets"], 3,
        "JSON should contain unverified_secrets"
    );
    assert_eq!(
        parsed["secrets_by_detector"]["AWS"], 2,
        "JSON should contain detector counts"
    );
    assert!(
        parsed["failed_repos"].is_array(),
        "JSON should contain failed_repos array"
    );
    assert_eq!(
        parsed["failed_repos"][0][0], "repo1",
        "JSON should contain failed repo name"
    );
}

#[test]
fn test_truffle_statistics_generate_detailed_report_empty() {
    let stats = TruffleStatistics::new();

    let report = stats
        .generate_detailed_report(false)
        .expect("Report generation should succeed");

    // Empty report should be empty or minimal
    assert!(
        report.trim().is_empty() || report.lines().count() < 3,
        "Empty stats should produce minimal report"
    );
}

#[test]
fn test_audit_statistics_initialization() {
    let stats = AuditStatistics::new();

    assert_eq!(
        stats.truffle_stats.total_repos_scanned, 0,
        "TruffleStats should be initialized"
    );
    // HygieneStatistics are private, but we verify the structure exists
}

#[test]
fn test_secret_finding_structure() {
    let finding = SecretFinding {
        detector_name: "AWS".to_string(),
        verified: true,
        file_path: "config/aws.yml".to_string(),
    };

    assert_eq!(finding.detector_name, "AWS", "Should store detector name");
    assert!(finding.verified, "Should store verification status");
    assert_eq!(
        finding.file_path, "config/aws.yml",
        "Should store file path"
    );
}

// =====================================================================================
// hygiene.rs tests - Repository hygiene checking
// =====================================================================================

#[test]
fn test_hygiene_statistics_initialization() {
    let stats = HygieneStatistics::new();

    assert_eq!(
        stats.get_violation_repos().len(),
        0,
        "Should start with no violations"
    );
}

// Note: HygieneStatus.symbol() and .text() are private methods used internally
// They are tested indirectly through the update() and generate_summary() methods

#[test]
fn test_hygiene_statistics_update_clean() {
    let mut stats = HygieneStatistics::new();

    stats.update(
        "clean-repo",
        "/path/to/clean-repo",
        &HygieneStatus::Clean,
        "no violations found",
        vec![],
    );

    let summary = stats.generate_summary(1, Duration::from_secs(5));
    assert!(summary.contains("1 clean"), "Should show clean repo count");
    assert!(
        summary.contains("0 with violations"),
        "Should show 0 violations"
    );
}

#[test]
fn test_hygiene_statistics_update_with_violations() {
    let mut stats = HygieneStatistics::new();

    let violations = vec![
        HygieneViolation {
            file_path: "node_modules/package/index.js".to_string(),
            violation_type: ViolationType::UniversalBadPattern,
            size_bytes: None,
        },
        HygieneViolation {
            file_path: ".env".to_string(),
            violation_type: ViolationType::GitignoreViolation,
            size_bytes: None,
        },
        HygieneViolation {
            file_path: "large_file.bin".to_string(),
            violation_type: ViolationType::LargeFile,
            size_bytes: Some(5_000_000),
        },
    ];

    stats.update(
        "dirty-repo",
        "/path/to/dirty-repo",
        &HygieneStatus::Violations,
        "3 violations found",
        violations,
    );

    let summary = stats.generate_summary(1, Duration::from_secs(10));
    assert!(summary.contains("0 clean"), "Should show 0 clean repos");
    assert!(
        summary.contains("1 with violations"),
        "Should show 1 repo with violations"
    );

    let violation_repos = stats.get_violation_repos();
    assert_eq!(
        violation_repos.len(),
        1,
        "Should track 1 repo with violations"
    );
    assert_eq!(violation_repos[0].0, "dirty-repo", "Should track repo name");
    assert_eq!(violation_repos[0].2.len(), 3, "Should track all violations");
}

#[test]
fn test_hygiene_statistics_update_with_error() {
    let mut stats = HygieneStatistics::new();

    stats.update(
        "error-repo",
        "/path/to/error-repo",
        &HygieneStatus::Error,
        "gitignore check failed: permission denied",
        vec![],
    );

    let summary = stats.generate_summary(1, Duration::from_secs(3));
    assert!(summary.contains("0 clean"), "Should show 0 clean repos");
    assert!(
        summary.contains("0 with violations"),
        "Should show 0 violation repos"
    );
    assert!(summary.contains("1 failed"), "Should show 1 failed repo");
}

#[test]
fn test_hygiene_statistics_multiple_repos() {
    let mut stats = HygieneStatistics::new();

    // Clean repo
    stats.update(
        "repo1",
        "/path/repo1",
        &HygieneStatus::Clean,
        "clean",
        vec![],
    );

    // Repo with violations
    let violations = vec![HygieneViolation {
        file_path: ".env".to_string(),
        violation_type: ViolationType::GitignoreViolation,
        size_bytes: None,
    }];
    stats.update(
        "repo2",
        "/path/repo2",
        &HygieneStatus::Violations,
        "violations",
        violations,
    );

    // Error repo
    stats.update(
        "repo3",
        "/path/repo3",
        &HygieneStatus::Error,
        "error",
        vec![],
    );

    // Another clean repo
    stats.update(
        "repo4",
        "/path/repo4",
        &HygieneStatus::Clean,
        "clean",
        vec![],
    );

    let summary = stats.generate_summary(4, Duration::from_secs(15));
    assert!(summary.contains("2 clean"), "Should count clean repos");
    assert!(
        summary.contains("1 with violations"),
        "Should count repos with violations"
    );
    assert!(summary.contains("1 failed"), "Should count failed repos");
}

#[test]
fn test_hygiene_statistics_generate_detailed_summary_with_violations() {
    let mut stats = HygieneStatistics::new();

    let violations = vec![
        HygieneViolation {
            file_path: "node_modules/pkg/index.js".to_string(),
            violation_type: ViolationType::UniversalBadPattern,
            size_bytes: None,
        },
        HygieneViolation {
            file_path: ".env".to_string(),
            violation_type: ViolationType::GitignoreViolation,
            size_bytes: None,
        },
        HygieneViolation {
            file_path: "big.bin".to_string(),
            violation_type: ViolationType::LargeFile,
            size_bytes: Some(2_000_000),
        },
    ];

    stats.update(
        "test-repo",
        "/very/long/path/to/test-repo",
        &HygieneStatus::Violations,
        "violations found",
        violations,
    );

    let detailed = stats.generate_detailed_summary();

    assert!(
        detailed.contains("HYGIENE VIOLATIONS"),
        "Should have violations header"
    );
    assert!(detailed.contains("test-repo"), "Should show repo name");
    assert!(detailed.contains("3 violations"), "Should show total count");
    assert!(
        detailed.contains("1 gitignore"),
        "Should show gitignore count"
    );
    assert!(detailed.contains("1 patterns"), "Should show pattern count");
    assert!(detailed.contains("1 large"), "Should show large file count");
}

#[test]
fn test_hygiene_statistics_generate_detailed_summary_with_failures() {
    let mut stats = HygieneStatistics::new();

    stats.update(
        "failed-repo",
        "/path/to/failed",
        &HygieneStatus::Error,
        "large file check failed: timeout",
        vec![],
    );

    let detailed = stats.generate_detailed_summary();

    assert!(
        detailed.contains("FAILED HYGIENE SCANS"),
        "Should have failures header"
    );
    assert!(
        detailed.contains("failed-repo"),
        "Should show failed repo name"
    );
    assert!(detailed.contains("timeout"), "Should show error message");
}

#[test]
fn test_hygiene_statistics_generate_detailed_summary_empty() {
    let stats = HygieneStatistics::new();

    let detailed = stats.generate_detailed_summary();

    assert!(
        detailed.is_empty(),
        "Empty stats should produce empty detailed summary"
    );
}

#[test]
fn test_hygiene_violation_types() {
    let gitignore_violation = HygieneViolation {
        file_path: ".env".to_string(),
        violation_type: ViolationType::GitignoreViolation,
        size_bytes: None,
    };

    let pattern_violation = HygieneViolation {
        file_path: "node_modules/pkg/file.js".to_string(),
        violation_type: ViolationType::UniversalBadPattern,
        size_bytes: None,
    };

    let large_violation = HygieneViolation {
        file_path: "huge.bin".to_string(),
        violation_type: ViolationType::LargeFile,
        size_bytes: Some(10_000_000),
    };

    assert_eq!(
        gitignore_violation.file_path, ".env",
        "Should store gitignore violation"
    );
    assert_eq!(
        pattern_violation.file_path, "node_modules/pkg/file.js",
        "Should store pattern violation"
    );
    assert_eq!(
        large_violation.size_bytes,
        Some(10_000_000),
        "Should store size for large files"
    );
}

// =====================================================================================
// fixes.rs tests - Automated fix operations
// =====================================================================================

#[test]
fn test_fix_options_fix_all() {
    let options = FixOptions::fix_all(false, None);

    assert!(!options.interactive, "fix_all should not be interactive");
    assert!(options.fix_gitignore, "fix_all should fix gitignore");
    assert!(options.fix_large, "fix_all should fix large files");
    assert!(options.fix_secrets, "fix_all should fix secrets");
    assert!(options.untrack_files, "fix_all should untrack files");
    assert!(!options.dry_run, "Should not be dry run by default");
    assert!(
        options.target_repos.is_none(),
        "Should not have target repos by default"
    );
}

#[test]
fn test_fix_options_fix_all_with_dry_run() {
    let options = FixOptions::fix_all(true, None);

    assert!(options.dry_run, "Should enable dry run when requested");
}

#[test]
fn test_fix_options_fix_all_with_target_repos() {
    let targets = vec!["repo1".to_string(), "repo2".to_string()];
    let options = FixOptions::fix_all(false, Some(targets.clone()));

    assert!(options.target_repos.is_some(), "Should have target repos");
    assert_eq!(
        options.target_repos.unwrap(),
        targets,
        "Should store target repos"
    );
}

#[test]
fn test_fix_options_selective() {
    let mut options = FixOptions::fix_all(false, None);
    options.fix_gitignore = true;
    options.fix_large = false;
    options.fix_secrets = false;
    options.untrack_files = false;

    assert!(
        options.fix_gitignore,
        "Should allow selective gitignore fix"
    );
    assert!(!options.fix_large, "Should allow disabling large file fix");
    assert!(!options.fix_secrets, "Should allow disabling secret fix");
    assert!(!options.untrack_files, "Should allow disabling untrack");
}

// =====================================================================================
// Integration tests - Testing with real git repositories
// =====================================================================================

#[tokio::test]
async fn test_hygiene_statistics_with_git_repo() {
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let repo = match TestRepoBuilder::new("test-hygiene-repo").build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Create a test violation scenario
    let _ = repo.create_file(".env", "SECRET_KEY=abc123");
    let _ = repo.commit_all("Add .env file");

    // Test that we can track violations from real repos
    let mut stats = HygieneStatistics::new();

    let violations = vec![HygieneViolation {
        file_path: ".env".to_string(),
        violation_type: ViolationType::GitignoreViolation,
        size_bytes: None,
    }];

    stats.update(
        "test-hygiene-repo",
        repo.path().to_str().unwrap(),
        &HygieneStatus::Violations,
        "1 violation",
        violations,
    );

    let violation_repos = stats.get_violation_repos();
    assert_eq!(
        violation_repos.len(),
        1,
        "Should track violation from git repo"
    );
}

#[tokio::test]
async fn test_truffle_statistics_with_multiple_detectors() {
    let mut stats = TruffleStatistics::new();

    // Simulate findings from multiple detector types
    let detectors = vec!["AWS", "GitHub", "Slack", "JWT", "PrivateKey"];

    for detector in &detectors {
        let secrets = vec![SecretFinding {
            detector_name: detector.to_string(),
            verified: false,
            file_path: format!("config/{}.yml", detector.to_lowercase()),
        }];
        stats.add_repo_result(&format!("{}-repo", detector), &secrets);
    }

    assert_eq!(stats.total_repos_scanned, 5, "Should scan all repos");
    assert_eq!(stats.repos_with_secrets, 5, "All repos have secrets");
    assert_eq!(
        stats.secrets_by_detector.len(),
        5,
        "Should track all detector types"
    );

    for detector in &detectors {
        assert_eq!(
            *stats.secrets_by_detector.get(*detector).unwrap(),
            1,
            "Should count secret for {} detector",
            detector
        );
    }
}

#[test]
fn test_truffle_statistics_report_sorting() {
    let mut stats = TruffleStatistics::new();

    // Add secrets with different counts (should sort by count descending)
    stats.secrets_by_detector.insert("AWS".to_string(), 10);
    stats.secrets_by_detector.insert("GitHub".to_string(), 5);
    stats.secrets_by_detector.insert("Slack".to_string(), 15);
    stats.verified_secrets = 30;

    let report = stats
        .generate_detailed_report(false)
        .expect("Report should succeed");

    // Check that detectors are sorted by count (descending)
    let lines: Vec<&str> = report.lines().collect();
    let detector_lines: Vec<&str> = lines
        .iter()
        .filter(|l| l.contains(" × "))
        .copied()
        .collect();

    // First should be Slack (15), then AWS (10), then GitHub (5)
    assert!(
        detector_lines[0].contains("15 × Slack"),
        "Should sort by count - Slack first"
    );
    assert!(
        detector_lines[1].contains("10 × AWS"),
        "Should sort by count - AWS second"
    );
    assert!(
        detector_lines[2].contains("5 × GitHub"),
        "Should sort by count - GitHub third"
    );
}

#[test]
fn test_hygiene_statistics_duration_formatting() {
    let stats = HygieneStatistics::new();

    // Test various durations
    let summary1 = stats.generate_summary(5, Duration::from_secs_f64(1.5));
    assert!(
        summary1.contains("1.5s"),
        "Should format sub-second duration"
    );

    let summary2 = stats.generate_summary(5, Duration::from_secs_f64(45.123));
    assert!(summary2.contains("45.1s"), "Should format with one decimal");

    let summary3 = stats.generate_summary(5, Duration::from_secs_f64(120.9));
    assert!(summary3.contains("120.9s"), "Should format long duration");
}

#[test]
fn test_comprehensive_audit_workflow() {
    // Simulate a complete audit workflow
    let mut truffle_stats = TruffleStatistics::new();
    let mut hygiene_stats = HygieneStatistics::new();

    // Repo 1: Clean
    truffle_stats.add_repo_result("clean-repo", &[]);
    hygiene_stats.update(
        "clean-repo",
        "/path/clean",
        &HygieneStatus::Clean,
        "clean",
        vec![],
    );

    // Repo 2: Has secrets and hygiene issues
    let secrets = vec![SecretFinding {
        detector_name: "AWS".to_string(),
        verified: true,
        file_path: ".env".to_string(),
    }];
    truffle_stats.add_repo_result("dirty-repo", &secrets);

    let violations = vec![
        HygieneViolation {
            file_path: ".env".to_string(),
            violation_type: ViolationType::GitignoreViolation,
            size_bytes: None,
        },
        HygieneViolation {
            file_path: "node_modules/pkg/index.js".to_string(),
            violation_type: ViolationType::UniversalBadPattern,
            size_bytes: None,
        },
    ];
    hygiene_stats.update(
        "dirty-repo",
        "/path/dirty",
        &HygieneStatus::Violations,
        "violations",
        violations,
    );

    // Repo 3: Scan failed
    truffle_stats.add_repo_failure("error-repo", "timeout");
    hygiene_stats.update(
        "error-repo",
        "/path/error",
        &HygieneStatus::Error,
        "error",
        vec![],
    );

    // Verify combined statistics
    assert_eq!(
        truffle_stats.total_repos_scanned, 3,
        "Should scan all repos"
    );
    assert_eq!(
        truffle_stats.verified_secrets, 1,
        "Should find verified secret"
    );

    let hygiene_summary = hygiene_stats.generate_summary(3, Duration::from_secs(20));
    assert!(
        hygiene_summary.contains("1 clean"),
        "Should have clean repo"
    );
    assert!(
        hygiene_summary.contains("1 with violations"),
        "Should have violations"
    );
    assert!(hygiene_summary.contains("1 failed"), "Should have failure");

    // Generate reports
    let truffle_report = truffle_stats
        .generate_detailed_report(false)
        .expect("Truffle report should generate");
    assert!(
        truffle_report.contains("VERIFIED SECRETS"),
        "Should highlight verified secrets"
    );
    assert!(
        truffle_report.contains("SCAN FAILURES"),
        "Should show failures"
    );

    let hygiene_report = hygiene_stats.generate_detailed_summary();
    assert!(
        hygiene_report.contains("HYGIENE VIOLATIONS"),
        "Should show violations"
    );
    assert!(
        hygiene_report.contains("FAILED HYGIENE SCANS"),
        "Should show failures"
    );
}
