//! Secret-history rewriting and repository safety checks.

use super::{ensure_command_success, write_private_temp_file, FixOptions};
use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::path::Path;
use tokio::process::Command;

pub(super) async fn fix_secrets_in_history(
    repo_path: &str,
    options: &FixOptions,
) -> Result<String> {
    if options.dry_run {
        return Ok("[DRY RUN] Would scan and remove secrets from history".to_string());
    }
    if !check_filter_repo_installed().await {
        return Err(anyhow!(
            "git-filter-repo is required to remove secrets. Please install it:\n\
             pip install git-filter-repo\n\
             or: brew install git-filter-repo (macOS)"
        ));
    }

    let output = Command::new("trufflehog")
        .args([
            "git",
            &format!("file://{repo_path}"),
            "--results=verified,unknown",
            "--json",
            "--no-update",
        ])
        .current_dir(repo_path)
        .output()
        .await?;
    if !output.status.success() {
        return Err(anyhow!(
            "TruffleHog scan failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut secret_files = HashSet::new();
    let mut secret_patterns = HashSet::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(file) = json["SourceMetadata"]["Data"]["Git"]["file"].as_str() {
                secret_files.insert(file.to_string());
            }
            if let Some(raw) = json["Raw"].as_str() {
                if raw.len() < 200 {
                    secret_patterns.insert(raw.to_string());
                }
            }
        }
    }

    if secret_files.is_empty() && secret_patterns.is_empty() {
        return Ok("No secrets found".to_string());
    }

    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup_ref = format!("refs/original/pre-fix-backup-secrets-{timestamp}");
    let backup_output = Command::new("git")
        .args(["update-ref", &backup_ref, "HEAD"])
        .current_dir(repo_path)
        .output()
        .await?;
    ensure_command_success(&backup_output, "creating secret-removal backup ref")?;

    eprintln!("    Backup created at: {backup_ref}");
    eprintln!("    Found {} files with secrets", secret_files.len());

    let mut replacements_content = String::new();
    for pattern in &secret_patterns {
        replacements_content.push_str(&format!("{pattern}==>REDACTED\n"));
    }

    if !replacements_content.is_empty() {
        let replacements_file =
            write_private_temp_file("filter-repo-secrets", &replacements_content)?;
        let result = Command::new("git")
            .args([
                "filter-repo",
                "--replace-text",
                replacements_file.path_str()?,
                "--force",
            ])
            .current_dir(repo_path)
            .output()
            .await?;
        if !result.status.success() {
            return Err(anyhow!(
                "git filter-repo failed: {}",
                String::from_utf8_lossy(&result.stderr)
            ));
        }
    }

    if !secret_files.is_empty() && secret_patterns.is_empty() {
        eprintln!("    Removing entire files containing secrets...");
        let paths_content: String = secret_files
            .iter()
            .map(|file| format!("literal:{file}\n"))
            .collect();
        let paths_file = write_private_temp_file("filter-repo-secret-files", &paths_content)?;
        let result = Command::new("git")
            .args([
                "filter-repo",
                "--invert-paths",
                "--paths-from-file",
                paths_file.path_str()?,
                "--force",
            ])
            .current_dir(repo_path)
            .output()
            .await?;
        if !result.status.success() {
            return Err(anyhow!(
                "git filter-repo failed: {}",
                String::from_utf8_lossy(&result.stderr)
            ));
        }
    }

    let gc_output = Command::new("git")
        .args(["gc", "--prune=now", "--aggressive"])
        .current_dir(repo_path)
        .output()
        .await?;
    ensure_command_success(&gc_output, "garbage collection")?;

    Ok(format!(
        "Removed/redacted {} secrets from history\n    Recovery: git reset --hard {}",
        secret_patterns.len() + secret_files.len(),
        backup_ref
    ))
}

pub(super) async fn check_repository_safety(repo_path: &str, options: &FixOptions) -> Result<()> {
    use crate::git::operations::run_git;

    if options.dry_run {
        return Ok(());
    }

    let repo_path_ref = Path::new(repo_path);
    let status = run_git(repo_path_ref, &["status", "--porcelain"]).await?;
    if !status.0 {
        return Err(anyhow!("git status failed: {}", status.2));
    }
    if !status.1.is_empty() {
        return Err(anyhow!(
            "Repository has uncommitted changes:\n{}\n\n\
             Please commit or stash changes before running fixes.\n\
             Run: git stash push -m \"Before repos fix\"",
            status.1
        ));
    }

    if options.fix_large || options.fix_secrets {
        let remotes = run_git(repo_path_ref, &["remote"]).await?;
        if !remotes.0 {
            return Err(anyhow!("remote inspection failed: {}", remotes.2));
        }
        if remotes.1.trim().is_empty() {
            return Ok(());
        }

        let upstream =
            run_git(repo_path_ref, &["rev-parse", "--abbrev-ref", "@{upstream}"]).await?;
        if !upstream.0 {
            return Err(anyhow!(
                "History rewrite requires a configured upstream: {}",
                upstream.2
            ));
        }

        let fetch = run_git(repo_path_ref, &["fetch", "--quiet"]).await?;
        if !fetch.0 {
            return Err(anyhow!("git fetch failed: {}", fetch.2));
        }

        let counts = crate::git::ancestry::ahead_behind(repo_path_ref).await?;
        if counts.behind > 0 {
            return Err(anyhow!(
                "Repository is {} commits behind remote.\nPull changes first: git pull",
                counts.behind
            ));
        }
        if counts.ahead > 0 {
            eprintln!(
                "⚠️  Warning: Repository is {} commits ahead of remote.\n   \
                 After history rewrite, you'll need: git push --force-with-lease",
                counts.ahead
            );
        }
    }

    Ok(())
}

pub(super) async fn check_filter_repo_installed() -> bool {
    Command::new("git")
        .args(["filter-repo", "--version"])
        .output()
        .await
        .map(|output| output.status.success())
        .unwrap_or(false)
}
