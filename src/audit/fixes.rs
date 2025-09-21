//! Fix operations for resolving hygiene violations in Git repositories
//!
//! This module provides functionality to fix common Git hygiene issues:
//! - Adding entries to .gitignore and untracking files
//! - Removing large files from Git history
//! - Removing secrets from Git history

use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use tokio::process::Command;
use serde_json;

use super::hygiene::{HygieneStatistics, HygieneViolation, ViolationType};

/// Configuration options for fix operations
#[derive(Debug, Clone)]
pub struct FixOptions {
    /// Interactive mode - prompt for each action
    pub interactive: bool,
    /// Fix .gitignore violations
    pub fix_gitignore: bool,
    /// Remove large files from history
    pub fix_large: bool,
    /// Remove secrets from history
    pub fix_secrets: bool,
    /// Also untrack files when fixing gitignore (not included in auto-fix)
    pub untrack_files: bool,
    /// Preview changes without applying them
    pub dry_run: bool,
    /// Only apply fixes to specific repositories
    pub target_repos: Option<Vec<String>>,
}

impl FixOptions {
    /// Create options for fix-all mode (apply all available fixes)
    pub fn fix_all(dry_run: bool, target_repos: Option<Vec<String>>) -> Self {
        Self {
            interactive: false,
            fix_gitignore: true,
            fix_large: true,
            fix_secrets: true,
            untrack_files: true,
            dry_run,
            target_repos,
        }
    }
}

/// Result of a fix operation
#[derive(Debug)]
pub struct FixResult {
    pub repo_name: String,
    pub fixes_applied: Vec<String>,
    pub errors: Vec<String>,
}

/// Apply fixes based on hygiene scan results
pub async fn apply_fixes(
    hygiene_stats: &HygieneStatistics,
    options: FixOptions,
) -> Result<Vec<FixResult>> {
    let mut results = Vec::new();

    // Get violation repos from statistics
    let violation_repos = hygiene_stats.get_violation_repos();

    // Filter repos if specific targets are specified
    let repos_to_fix: Vec<_> = if let Some(ref targets) = options.target_repos {
        violation_repos
            .into_iter()
            .filter(|(name, _, _)| targets.contains(name))
            .collect()
    } else {
        violation_repos
    };

    if repos_to_fix.is_empty() {
        if options.target_repos.is_some() {
            println!("\n❌ No violations found in specified repositories");
        } else {
            println!("\n✅ No hygiene violations to fix!");
        }
        return Ok(results);
    }

    // Show summary and get confirmation if interactive
    if options.interactive || !options.dry_run {
        show_fix_summary(&repos_to_fix, &options).await?;

        if !options.dry_run && !confirm_fixes(&options).await? {
            println!("\n❌ Fix operation cancelled");
            return Ok(results);
        }
    }

    // Safety check: Verify git status before proceeding
    println!("\n🔍 Performing safety checks...");
    for (repo_name, repo_path, _) in &repos_to_fix {
        if let Err(e) = check_repository_safety(repo_path, &options).await {
            println!("❌ Safety check failed for {}: {}", repo_name, e);
            return Err(e);
        }
    }
    println!("✅ All repositories passed safety checks\n");

    println!("🧹 Applying fixes to {} repositories...\n", repos_to_fix.len());

    // Process each repository
    for (repo_name, repo_path, violations) in repos_to_fix {
        let mut result = FixResult {
            repo_name: repo_name.clone(),
            fixes_applied: Vec::new(),
            errors: Vec::new(),
        };

        println!("Processing {}...", repo_name);

        // Apply gitignore fixes
        if options.fix_gitignore {
            match fix_gitignore_violations(&repo_path, &violations, &options).await {
                Ok(msg) => {
                    if !msg.is_empty() {
                        println!("  ✓ {}", msg);
                        result.fixes_applied.push(msg);
                    }
                }
                Err(e) => {
                    let error_msg = format!("gitignore fix failed: {}", e);
                    println!("  ✗ {}", error_msg);
                    result.errors.push(error_msg);
                }
            }
        }

        // Apply large file fixes
        if options.fix_large {
            match fix_large_files(&repo_path, &violations, &options).await {
                Ok(msg) => {
                    if !msg.is_empty() {
                        println!("  ✓ {}", msg);
                        result.fixes_applied.push(msg);
                    }
                }
                Err(e) => {
                    let error_msg = format!("large file fix failed: {}", e);
                    println!("  ✗ {}", error_msg);
                    result.errors.push(error_msg);
                }
            }
        }

        // Apply secret fixes
        if options.fix_secrets {
            match fix_secrets_in_history(&repo_path, &options).await {
                Ok(msg) => {
                    if !msg.is_empty() {
                        println!("  ✓ {}", msg);
                        result.fixes_applied.push(msg);
                    }
                }
                Err(e) => {
                    let error_msg = format!("secret removal failed: {}", e);
                    println!("  ✗ {}", error_msg);
                    result.errors.push(error_msg);
                }
            }
        }

        results.push(result);
    }

    // Show final summary
    show_fix_results(&results);

    Ok(results)
}

/// Show summary of fixes to be applied
async fn show_fix_summary(
    repos: &[(String, String, Vec<HygieneViolation>)],
    options: &FixOptions,
) -> Result<()> {
    println!("\n📋 Fix Summary\n");

    let mut total_gitignore = 0;
    let mut total_large = 0;
    let mut total_patterns = 0;

    for (_, _, violations) in repos {
        for violation in violations {
            match violation.violation_type {
                ViolationType::GitignoreViolation => total_gitignore += 1,
                ViolationType::LargeFile => total_large += 1,
                ViolationType::UniversalBadPattern => total_patterns += 1,
            }
        }
    }

    println!("Found violations in {} repositories:", repos.len());

    if options.fix_gitignore {
        println!("  📝 {} files need .gitignore entries", total_gitignore + total_patterns);
        if options.untrack_files {
            println!("     → Will untrack files after adding to .gitignore");
        } else {
            println!("     → Will only add to .gitignore (files remain tracked)");
        }
    }

    if options.fix_large {
        println!("  📦 {} large files in history", total_large);
        println!("     → Will remove from Git history (requires force-push)");
    }

    if options.fix_secrets {
        println!("  🔑 Secrets will be scanned and removed");
        println!("     → Will rewrite Git history (requires force-push)");
    }

    if options.dry_run {
        println!("\n⚠️  DRY RUN MODE - No changes will be made");
    }

    Ok(())
}

/// Prompt user for confirmation
async fn confirm_fixes(options: &FixOptions) -> Result<bool> {
    if options.dry_run {
        return Ok(true); // Always proceed in dry-run mode
    }

    println!("\n═══════════════════════════════════════════════════════════════════");
    println!("⚠️  CONFIRMATION REQUIRED");
    println!("═══════════════════════════════════════════════════════════════════");

    if options.fix_large || options.fix_secrets {
        println!("\n🔴 DESTRUCTIVE OPERATION - HISTORY REWRITE");
        println!("   • Git history will be permanently rewritten");
        println!("   • Backups saved in refs/original/pre-fix-backup-*");
        println!("   • git filter-repo also saves refs in .git/filter-repo/");
        println!("   • You will need to force-push: git push --force-with-lease");
        println!("   • All collaborators must re-clone or reset their branches");
        println!("\n   ROLLBACK: git reset --hard refs/original/pre-fix-backup-<timestamp>");
    } else if options.untrack_files {
        println!("\n🟡 MODERATE OPERATION - FILE UNTRACKING");
        println!("   • Files will be removed from Git tracking");
        println!("   • Files remain in your working directory");
        println!("   • Changes are reversible with: git add <files>");
    } else {
        println!("\n🟢 SAFE OPERATION - GITIGNORE UPDATE");
        println!("   • Only .gitignore files will be modified");
        println!("   • Files remain tracked until manually untracked");
        println!("   • Changes are easily reversible");
    }

    println!("\n═══════════════════════════════════════════════════════════════════");

    print!("\nType 'yes' to proceed or anything else to cancel: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(input.trim().to_lowercase() == "yes")
}

/// Fix .gitignore violations for a repository
async fn fix_gitignore_violations(
    repo_path: &str,
    violations: &[HygieneViolation],
    options: &FixOptions,
) -> Result<String> {
    let gitignore_violations: Vec<_> = violations
        .iter()
        .filter(|v| matches!(
            v.violation_type,
            ViolationType::GitignoreViolation | ViolationType::UniversalBadPattern
        ))
        .collect();

    if gitignore_violations.is_empty() {
        return Ok(String::new());
    }

    if options.dry_run {
        return Ok(format!(
            "[DRY RUN] Would add {} entries to .gitignore",
            gitignore_violations.len()
        ));
    }

    // Group patterns intelligently
    let patterns = group_gitignore_patterns(&gitignore_violations);

    // Read existing .gitignore
    let gitignore_path = Path::new(repo_path).join(".gitignore");
    let existing_content = fs::read_to_string(&gitignore_path).unwrap_or_default();
    let existing_patterns: HashSet<_> = existing_content
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .collect();

    // Add new patterns
    let mut new_patterns = Vec::new();
    for pattern in patterns {
        if !existing_patterns.contains(pattern.as_str()) {
            new_patterns.push(pattern);
        }
    }

    if new_patterns.is_empty() {
        return Ok("All patterns already in .gitignore".to_string());
    }

    // Append to .gitignore
    let mut gitignore_content = existing_content;
    if !gitignore_content.ends_with('\n') && !gitignore_content.is_empty() {
        gitignore_content.push('\n');
    }

    gitignore_content.push_str("\n# Added by repos audit --fix-gitignore\n");
    for pattern in &new_patterns {
        gitignore_content.push_str(pattern);
        gitignore_content.push('\n');
    }

    fs::write(&gitignore_path, gitignore_content)?;

    // Untrack files if requested
    let mut untracked_count = 0;
    if options.untrack_files {
        for violation in gitignore_violations {
            let result = Command::new("git")
                .args(&["rm", "--cached", "-r", &violation.file_path])
                .current_dir(repo_path)
                .output()
                .await?;

            if result.status.success() {
                untracked_count += 1;
            }
        }
    }

    // Create commit
    Command::new("git")
        .args(&["add", ".gitignore"])
        .current_dir(repo_path)
        .output()
        .await?;

    let commit_message = if untracked_count > 0 {
        format!(
            "chore: Update .gitignore and untrack {} ignored files\n\nAdded {} patterns to .gitignore",
            untracked_count,
            new_patterns.len()
        )
    } else {
        format!(
            "chore: Update .gitignore\n\nAdded {} patterns to .gitignore",
            new_patterns.len()
        )
    };

    Command::new("git")
        .args(&["commit", "-m", &commit_message])
        .current_dir(repo_path)
        .output()
        .await?;

    Ok(format!(
        "Added {} patterns to .gitignore{}",
        new_patterns.len(),
        if untracked_count > 0 {
            format!(", untracked {} files", untracked_count)
        } else {
            String::new()
        }
    ))
}

/// Group gitignore patterns intelligently
fn group_gitignore_patterns(violations: &[&HygieneViolation]) -> Vec<String> {
    let mut patterns = HashMap::new();

    for violation in violations {
        let path = &violation.file_path;

        // Extract common patterns
        if path.contains("node_modules/") {
            patterns.insert("node_modules/", true);
        } else if path.contains("target/debug/") {
            patterns.insert("target/debug/", true);
        } else if path.contains("target/release/") {
            patterns.insert("target/release/", true);
        } else if path.contains("dist/") {
            patterns.insert("dist/", true);
        } else if path.contains("build/") {
            patterns.insert("build/", true);
        } else if path.contains("__pycache__/") {
            patterns.insert("__pycache__/", true);
        } else if path.contains(".venv/") {
            patterns.insert(".venv/", true);
        } else if path.ends_with(".log") {
            patterns.insert("*.log", true);
        } else if path.ends_with(".tmp") {
            patterns.insert("*.tmp", true);
        } else if path.ends_with(".cache") {
            patterns.insert("*.cache", true);
        } else if path == ".DS_Store" {
            patterns.insert(".DS_Store", true);
        } else if path == "Thumbs.db" {
            patterns.insert("Thumbs.db", true);
        } else if path == ".env" {
            patterns.insert(".env", true);
        } else if path.ends_with(".key") || path.ends_with(".pem") {
            patterns.insert("*.key", true);
            patterns.insert("*.pem", true);
        } else {
            // For specific files, add the exact path
            patterns.insert(path.as_str(), false);
        }
    }

    let mut result: Vec<String> = patterns
        .keys()
        .map(|&k| k.to_string())
        .collect();

    result.sort();
    result
}

/// Fix large files in Git history using git filter-repo
async fn fix_large_files(
    repo_path: &str,
    violations: &[HygieneViolation],
    options: &FixOptions,
) -> Result<String> {
    let large_files: Vec<_> = violations
        .iter()
        .filter(|v| matches!(v.violation_type, ViolationType::LargeFile))
        .collect();

    if large_files.is_empty() {
        return Ok(String::new());
    }

    if options.dry_run {
        let total_size: u64 = large_files
            .iter()
            .map(|f| f.size_bytes.unwrap_or(0))
            .sum();
        return Ok(format!(
            "[DRY RUN] Would remove {} large files ({:.1} MB total)",
            large_files.len(),
            total_size as f64 / 1_048_576.0
        ));
    }

    // Check if git filter-repo is installed
    if !check_filter_repo_installed().await {
        return Err(anyhow!(
            "git-filter-repo is required to remove large files. Please install it:\n\
             pip install git-filter-repo\n\
             or: brew install git-filter-repo (macOS)\n\
             or: apt install git-filter-repo (Ubuntu 22.04+)"
        ));
    }

    // git filter-repo automatically creates backup in .git/filter-repo/original/refs
    println!("    Creating backup before history rewrite...");

    // Create our own backup ref as well for extra safety
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup_ref = format!("refs/original/pre-fix-backup-large-{}", timestamp);

    Command::new("git")
        .args(&["update-ref", &backup_ref, "HEAD"])
        .current_dir(repo_path)
        .output()
        .await?;

    println!("    Backup created at: {}", backup_ref);

    // Create a paths file for filter-repo
    let paths_file = std::env::temp_dir().join(format!("filter-repo-paths-{}.txt", timestamp));
    let mut paths_content = String::new();

    for file in &large_files {
        let size_mb = file.size_bytes.unwrap_or(0) as f64 / 1_048_576.0;
        println!("    Removing {} ({:.1} MB) from history...", file.file_path, size_mb);
        // Add to paths file (literal paths for filter-repo)
        paths_content.push_str(&format!("literal:{}\n", file.file_path));
    }

    // Write paths to temporary file
    fs::write(&paths_file, paths_content)?;

    // Run git filter-repo to remove the files
    let result = Command::new("git")
        .args(&[
            "filter-repo",
            "--invert-paths",
            "--paths-from-file", paths_file.to_str().unwrap(),
            "--force",  // Required if there are existing remote refs
        ])
        .current_dir(repo_path)
        .output()
        .await?;

    // Clean up temp file
    let _ = fs::remove_file(paths_file);

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(anyhow!("git filter-repo failed: {}", stderr));
    }

    // Run garbage collection to reclaim space
    Command::new("git")
        .args(&["gc", "--prune=now", "--aggressive"])
        .current_dir(repo_path)
        .output()
        .await?;

    Ok(format!(
        "Removed {} large files from history\n    \
         Recovery: git reset --hard {}",
        large_files.len(),
        backup_ref
    ))
}

/// Fix secrets in Git history using git filter-repo
async fn fix_secrets_in_history(repo_path: &str, options: &FixOptions) -> Result<String> {
    if options.dry_run {
        return Ok("[DRY RUN] Would scan and remove secrets from history".to_string());
    }

    // Check if git filter-repo is installed
    if !check_filter_repo_installed().await {
        return Err(anyhow!(
            "git-filter-repo is required to remove secrets. Please install it:\n\
             pip install git-filter-repo\n\
             or: brew install git-filter-repo (macOS)"
        ));
    }

    // Run TruffleHog to get detailed secret information
    let output = Command::new("trufflehog")
        .args(&[
            "git",
            &format!("file://{}", repo_path),
            "--results=verified,unknown",
            "--json",
            "--no-update",  // Don't auto-update during scan
        ])
        .current_dir(repo_path)
        .output()
        .await?;

    if !output.status.success() && output.stdout.is_empty() {
        return Ok("No secrets found".to_string());
    }

    // Parse secrets from output to get file paths
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut secret_files = HashSet::new();
    let mut secret_patterns = HashSet::new();

    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }

        // Parse JSON to extract file paths and secret patterns
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            if let Some(file) = json["SourceMetadata"]["Data"]["Git"]["file"].as_str() {
                secret_files.insert(file.to_string());
            }
            // Extract the actual secret pattern to redact
            if let Some(raw) = json["Raw"].as_str() {
                // Only add shorter secrets as patterns (avoid adding entire files)
                if raw.len() < 200 {
                    secret_patterns.insert(raw.to_string());
                }
            }
        }
    }

    if secret_files.is_empty() && secret_patterns.is_empty() {
        return Ok("No secrets found".to_string());
    }

    // Create backup before rewriting history
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let backup_ref = format!("refs/original/pre-fix-backup-secrets-{}", timestamp);

    Command::new("git")
        .args(&["update-ref", &backup_ref, "HEAD"])
        .current_dir(repo_path)
        .output()
        .await?;

    println!("    Backup created at: {}", backup_ref);
    println!("    Found {} files with secrets", secret_files.len());

    // Create replacement file for filter-repo
    let replacements_file = std::env::temp_dir().join(format!("filter-repo-secrets-{}.txt", timestamp));
    let mut replacements_content = String::new();

    // Add patterns to be replaced with REDACTED
    for pattern in &secret_patterns {
        // Escape the pattern and replace with REDACTED
        replacements_content.push_str(&format!("{}==>REDACTED\n", pattern));
    }

    if !replacements_content.is_empty() {
        fs::write(&replacements_file, replacements_content)?;

        // Run git filter-repo to replace secrets with REDACTED
        let result = Command::new("git")
            .args(&[
                "filter-repo",
                "--replace-text", replacements_file.to_str().unwrap(),
                "--force",
            ])
            .current_dir(repo_path)
            .output()
            .await?;

        let _ = fs::remove_file(replacements_file);

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(anyhow!("git filter-repo failed: {}", stderr));
        }
    }

    // For files that contain secrets but patterns are too long, remove entire files
    if !secret_files.is_empty() && secret_patterns.is_empty() {
        println!("    Removing entire files containing secrets...");

        let paths_file = std::env::temp_dir().join(format!("filter-repo-secret-files-{}.txt", timestamp));
        let paths_content: String = secret_files
            .iter()
            .map(|f| format!("literal:{}\n", f))
            .collect();

        fs::write(&paths_file, paths_content)?;

        let result = Command::new("git")
            .args(&[
                "filter-repo",
                "--invert-paths",
                "--paths-from-file", paths_file.to_str().unwrap(),
                "--force",
            ])
            .current_dir(repo_path)
            .output()
            .await?;

        let _ = fs::remove_file(paths_file);

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            return Err(anyhow!("git filter-repo failed: {}", stderr));
        }
    }

    // Run garbage collection
    Command::new("git")
        .args(&["gc", "--prune=now", "--aggressive"])
        .current_dir(repo_path)
        .output()
        .await?;

    Ok(format!(
        "Removed/redacted {} secrets from history\n    \
         Recovery: git reset --hard {}",
        secret_patterns.len() + secret_files.len(),
        backup_ref
    ))
}

/// Check repository safety before applying fixes
async fn check_repository_safety(repo_path: &str, options: &FixOptions) -> Result<()> {
    // Skip checks in dry-run mode
    if options.dry_run {
        return Ok(());
    }

    // Check for uncommitted changes
    let status_output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .await?;

    if !status_output.stdout.is_empty() {
        let changes = String::from_utf8_lossy(&status_output.stdout);
        return Err(anyhow!(
            "Repository has uncommitted changes:\n{}\n\n\
             Please commit or stash changes before running fixes.\n\
             Run: git stash push -m \"Before repos fix\"",
            changes.trim()
        ));
    }

    // Check if we're up to date with remote (for operations that rewrite history)
    if options.fix_large || options.fix_secrets {
        // Fetch to ensure we have latest remote info
        Command::new("git")
            .args(["fetch", "--quiet"])
            .current_dir(repo_path)
            .output()
            .await?;

        // Check if we're behind remote
        let behind_output = Command::new("git")
            .args(["rev-list", "--count", "HEAD..@{u}"])
            .current_dir(repo_path)
            .output()
            .await?;

        if behind_output.status.success() {
            let behind_count = String::from_utf8_lossy(&behind_output.stdout)
                .trim()
                .parse::<u32>()
                .unwrap_or(0);

            if behind_count > 0 {
                return Err(anyhow!(
                    "Repository is {} commits behind remote.\n\
                     Pull changes first: git pull",
                    behind_count
                ));
            }
        }

        // Check if we're ahead (will need force push)
        let ahead_output = Command::new("git")
            .args(["rev-list", "--count", "@{u}..HEAD"])
            .current_dir(repo_path)
            .output()
            .await?;

        if ahead_output.status.success() {
            let ahead_count = String::from_utf8_lossy(&ahead_output.stdout)
                .trim()
                .parse::<u32>()
                .unwrap_or(0);

            if ahead_count > 0 {
                println!(
                    "⚠️  Warning: Repository is {} commits ahead of remote.\n   \
                     After history rewrite, you'll need: git push --force-with-lease",
                    ahead_count
                );
            }
        }
    }

    Ok(())
}

/// Check if git filter-repo is installed
async fn check_filter_repo_installed() -> bool {
    Command::new("git")
        .args(&["filter-repo", "--version"])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Show final results of fix operations
fn show_fix_results(results: &[FixResult]) {
    println!("\n═══════════════════════════════════════════════════════════════════");

    let successful = results.iter().filter(|r| r.errors.is_empty()).count();
    let failed = results.iter().filter(|r| !r.errors.is_empty()).count();

    println!("✅ Fix Summary: {} successful, {} failed", successful, failed);

    if failed > 0 {
        println!("\n⚠️  Failed fixes:");
        for result in results.iter().filter(|r| !r.errors.is_empty()) {
            println!("  {} - {}", result.repo_name, result.errors.join(", "));
        }
    }

    println!("═══════════════════════════════════════════════════════════════════");
}