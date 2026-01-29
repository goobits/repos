//! Hygiene scanning logic

use super::report::{HygieneStatus, HygieneViolation, ViolationType};
use super::rules::{LARGE_FILE_THRESHOLD, UNIVERSAL_BAD_PATTERNS};
use crate::core::config::{GIT_OBJECTS_CHUNK_SIZE, LARGE_FILES_DISPLAY_LIMIT};
use anyhow::Result;
use std::path::Path;
use tokio::process::Command;

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
                line.starts_with(pattern) || line.contains(&format!("/{pattern}"))
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
    for chunk in objects.chunks(GIT_OBJECTS_CHUNK_SIZE) {
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
    violations.truncate(LARGE_FILES_DISPLAY_LIMIT);

    Ok(violations)
}

/// Scans a repository for hygiene violations
pub async fn check_repo_hygiene(
    repo_path: &Path,
) -> (HygieneStatus, String, Vec<HygieneViolation>) {
    let mut all_violations = Vec::new();

    // Check gitignore violations
    match check_gitignore_violations(repo_path).await {
        Ok(mut violations) => all_violations.append(&mut violations),
        Err(e) => {
            return (
                HygieneStatus::Error,
                format!("gitignore check failed: {e}"),
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
                format!("pattern check failed: {e}"),
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
                format!("large file check failed: {e}"),
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
        let message = format!(
            "{}
 violations found",
            all_violations.len()
        );
        (HygieneStatus::Violations, message, all_violations)
    }
}
