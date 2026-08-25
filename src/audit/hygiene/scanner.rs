//! Hygiene scanning logic

use super::report::{HygieneStatus, HygieneViolation, ViolationType};
use super::rules::{LARGE_FILE_THRESHOLD, UNIVERSAL_BAD_PATTERNS};
use crate::core::config::LARGE_FILES_DISPLAY_LIMIT;
use anyhow::Result;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
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
        anyhow::bail!(
            "git ls-files ignored check failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
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
        anyhow::bail!(
            "git ls-files check failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
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
    let mut rev_list = Command::new("git")
        .args(["rev-list", "--objects", "--all"])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let mut rev_list_stdout = rev_list
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("git history listing stdout was unavailable"))?;
    let mut rev_list_stderr = rev_list
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("git history listing stderr was unavailable"))?;

    let mut cat_file = Command::new("git")
        .args(["cat-file", "--batch-check=%(objectsize) %(rest)"])
        .current_dir(repo_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let cat_file_stdout = cat_file
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("git object inspection stdout was unavailable"))?;
    let mut cat_file_stdin = cat_file
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("git object inspection stdin was unavailable"))?;
    let mut cat_file_stderr = cat_file
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("git object inspection stderr was unavailable"))?;

    let rev_stderr_task = tokio::spawn(async move {
        let mut stderr = Vec::new();
        rev_list_stderr
            .read_to_end(&mut stderr)
            .await
            .map(|_| stderr)
    });
    let object_pipe_task = tokio::spawn(async move {
        tokio::io::copy(&mut rev_list_stdout, &mut cat_file_stdin).await?;
        cat_file_stdin.shutdown().await
    });
    let cat_stderr_task = tokio::spawn(async move {
        let mut stderr = Vec::new();
        cat_file_stderr
            .read_to_end(&mut stderr)
            .await
            .map(|_| stderr)
    });

    let mut violations = Vec::new();
    let mut lines = BufReader::new(cat_file_stdout).lines();
    while let Some(line) = lines.next_line().await? {
        let Some((size, file_path)) = line.split_once(' ') else {
            continue;
        };
        let Ok(size) = size.parse::<u64>() else {
            continue;
        };
        if size > LARGE_FILE_THRESHOLD && !file_path.is_empty() {
            violations.push(HygieneViolation {
                file_path: file_path.to_string(),
                violation_type: ViolationType::LargeFile,
                size_bytes: Some(size),
            });
            violations.sort_by_key(|violation| {
                std::cmp::Reverse(violation.size_bytes.unwrap_or_default())
            });
            violations.truncate(LARGE_FILES_DISPLAY_LIMIT);
        }
    }

    let cat_status = cat_file.wait().await?;
    let rev_status = rev_list.wait().await?;
    object_pipe_task.await??;
    let rev_stderr = rev_stderr_task.await??;
    let cat_stderr = cat_stderr_task.await??;
    if !rev_status.success() {
        anyhow::bail!(
            "git history listing failed: {}",
            String::from_utf8_lossy(&rev_stderr).trim()
        );
    }
    if !cat_status.success() {
        anyhow::bail!(
            "git object inspection failed: {}",
            String::from_utf8_lossy(&cat_stderr).trim()
        );
    }

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
