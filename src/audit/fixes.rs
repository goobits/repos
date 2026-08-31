//! Fix operations for resolving hygiene violations in Git repositories
//!
//! This module provides functionality to fix common Git hygiene issues:
//! - Adding entries to .gitignore and untracking files
//! - Removing large files from Git history
//! - Removing secrets from Git history

mod gitignore;
mod history;

use gitignore::fix_gitignore_violations;
use history::{check_repository_safety, rewrite_history};

use crate::core::{RepositoryOrder, RepositoryTopology};
use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

const HISTORY_PUBLICATION_GUIDANCE: &str =
    "Remote publication is not automated; an ordinary git push is insufficient. Review and publish every rewritten branch and tag.";

pub(super) fn ensure_command_success(output: &std::process::Output, operation: &str) -> Result<()> {
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(anyhow!("{operation} failed: {}", stderr.trim()))
    }
}

use super::hygiene::{HygieneStatistics, HygieneViolation, ViolationType};

struct PrivateTempFile {
    path: PathBuf,
}

impl Drop for PrivateTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn write_private_temp_file(prefix: &str, contents: &str) -> Result<PrivateTempFile> {
    let temp_dir = std::env::temp_dir();
    let pid = std::process::id();
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S");

    for attempt in 0..100 {
        let path = temp_dir.join(format!("{prefix}-{timestamp}-{pid}-{attempt}.txt"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);

        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        match options.open(&path) {
            Ok(mut file) => {
                file.write_all(contents.as_bytes())?;
                return Ok(PrivateTempFile { path });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }

    Err(anyhow!("Failed to create private temp file"))
}

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
    /// Skip confirmation prompts (for automation/tests)
    pub skip_confirm: bool,
}

impl FixOptions {
    /// Create options for fix-all mode (apply all available fixes)
    #[must_use]
    pub fn fix_all(dry_run: bool) -> Self {
        Self {
            interactive: false,
            fix_gitignore: true,
            fix_large: true,
            fix_secrets: true,
            untrack_files: true,
            dry_run,
            skip_confirm: false,
        }
    }

    fn for_repository(&self, repository: &FixRepository) -> Self {
        let mut options = self.clone();
        options.interactive = false;
        options.fix_gitignore = repository.fix_gitignore;
        options.fix_large = repository.fix_large;
        options.fix_secrets = repository.fix_secrets;
        options
    }
}

/// Result of a fix operation
#[derive(Debug, serde::Serialize)]
pub struct FixResult {
    pub repo_name: String,
    pub fixes_applied: Vec<String>,
    pub errors: Vec<String>,
}

struct FixRepository {
    name: String,
    path: PathBuf,
    violations: Vec<HygieneViolation>,
    fix_gitignore: bool,
    fix_large: bool,
    fix_secrets: bool,
}

/// Apply fixes based on hygiene scan results
pub async fn apply_fixes(
    repositories: &[(String, PathBuf)],
    truffle_stats: &super::scanner::TruffleStatistics,
    hygiene_stats: &HygieneStatistics,
    mut options: FixOptions,
) -> Result<Vec<FixResult>> {
    let mut results = Vec::new();

    if options.interactive && !select_interactive_fixes(hygiene_stats, truffle_stats, &mut options)?
    {
        eprintln!("\n❌ No fixes selected");
        return Ok(results);
    }

    let violation_repos = hygiene_stats
        .get_violation_repos()
        .into_iter()
        .map(|(_, path, violations)| (PathBuf::from(path), violations))
        .collect::<HashMap<_, _>>();
    let topology = RepositoryTopology::new(repositories);
    let mut candidates = HashMap::new();
    for (name, path) in repositories {
        let violations = violation_repos.get(path).cloned().unwrap_or_default();
        let fix_gitignore = options.fix_gitignore
            && violations.iter().any(|violation| {
                matches!(
                    violation.violation_type,
                    ViolationType::GitignoreViolation | ViolationType::UniversalBadPattern
                )
            });
        let fix_large = options.fix_large
            && violations
                .iter()
                .any(|violation| matches!(violation.violation_type, ViolationType::LargeFile));
        let fix_secrets = options.fix_secrets && truffle_stats.repository_has_secrets(path);
        if fix_gitignore || fix_large || fix_secrets {
            candidates.insert(
                path.clone(),
                FixRepository {
                    name: name.clone(),
                    path: path.clone(),
                    violations,
                    fix_gitignore,
                    fix_large,
                    fix_secrets,
                },
            );
        }
    }

    let has_history_rewrites = candidates
        .values()
        .any(|repo| repo.fix_large || repo.fix_secrets);
    if has_history_rewrites && topology.has_gitlink_inspection_failures() {
        anyhow::bail!("Cannot verify submodule relationships for history rewrite");
    }
    if has_history_rewrites && topology.has_gitlink_dependencies() {
        anyhow::bail!(
            "bulk history rewrite is unsafe across parent/submodule dependencies; target and rewrite each dependency chain explicitly"
        );
    }
    let repos_to_fix = topology
        .waves(RepositoryOrder::ChildrenFirst)
        .into_iter()
        .flatten()
        .filter_map(|index| candidates.remove(&repositories[index].1))
        .collect::<Vec<_>>();

    if repos_to_fix.is_empty() {
        eprintln!("\n✅ No selected audit findings to fix!");
        return Ok(results);
    }

    // Show summary and get confirmation if interactive
    if (options.interactive || !options.dry_run) && !options.skip_confirm {
        show_fix_summary(&repos_to_fix, &options).await?;

        if !options.dry_run && !confirm_fixes(&options).await? {
            eprintln!("\n❌ Fix operation cancelled");
            return Ok(results);
        }
    }

    // Safety check: Verify git status before proceeding
    eprintln!("\n🔍 Performing safety checks...");
    for repo in &repos_to_fix {
        let repo_options = options.for_repository(repo);
        if let Err(e) = check_repository_safety(&repo.path, &repo_options).await {
            eprintln!("❌ Safety check failed for {}: {e}", repo.name);
            return Err(e);
        }
    }
    eprintln!("✅ All repositories passed safety checks\n");

    eprintln!(
        "🧹 Applying fixes to {} repositories...\n",
        repos_to_fix.len()
    );

    // Process each repository
    for repo in repos_to_fix {
        let repo_options = options.for_repository(&repo);
        let mut result = FixResult {
            repo_name: repo.name.clone(),
            fixes_applied: Vec::new(),
            errors: Vec::new(),
        };

        eprintln!("Processing {}...", repo.name);

        // The fleet-wide check above gives an early all-or-nothing gate. Repeat
        // it at the mutation boundary so a shared checkout cannot go stale.
        if let Err(error) = check_repository_safety(&repo.path, &repo_options).await {
            let error = format!("safety recheck failed: {error}");
            eprintln!("  ✗ {error}");
            result.errors.push(error);
            results.push(result);
            continue;
        }

        // Apply gitignore fixes
        if repo.fix_gitignore {
            match fix_gitignore_violations(&repo.path, &repo.violations, &repo_options).await {
                Ok(msg) => {
                    if !msg.is_empty() {
                        eprintln!("  ✓ {msg}");
                        result.fixes_applied.push(msg);
                    }
                }
                Err(e) => {
                    let error_msg = format!("gitignore fix failed: {e}");
                    eprintln!("  ✗ {error_msg}");
                    result.errors.push(error_msg);
                    results.push(result);
                    continue;
                }
            }
        }

        // Apply all selected history changes in one planned rewrite.
        if repo.fix_large || repo.fix_secrets {
            match rewrite_history(&repo.path, &repo_options).await {
                Ok(msg) => {
                    if !msg.is_empty() {
                        eprintln!("  ✓ {msg}");
                        result.fixes_applied.push(msg);
                    }
                }
                Err(e) => {
                    let error_msg = format!("history rewrite failed: {e}");
                    eprintln!("  ✗ {error_msg}");
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

fn select_interactive_fixes(
    hygiene_stats: &HygieneStatistics,
    truffle_stats: &super::scanner::TruffleStatistics,
    options: &mut FixOptions,
) -> Result<bool> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stderr = io::stderr();
    let mut output = stderr.lock();
    select_interactive_fixes_with_io(
        hygiene_stats,
        truffle_stats,
        options,
        &mut input,
        &mut output,
    )
}

fn select_interactive_fixes_with_io<R: BufRead, W: Write>(
    hygiene_stats: &HygieneStatistics,
    truffle_stats: &super::scanner::TruffleStatistics,
    options: &mut FixOptions,
    input: &mut R,
    output: &mut W,
) -> Result<bool> {
    let violations = hygiene_stats
        .get_violation_repos()
        .into_iter()
        .flat_map(|(_, _, violations)| violations)
        .collect::<Vec<_>>();
    let has_gitignore = violations.iter().any(|violation| {
        matches!(
            violation.violation_type,
            ViolationType::GitignoreViolation | ViolationType::UniversalBadPattern
        )
    });
    let has_large = violations
        .iter()
        .any(|violation| matches!(violation.violation_type, ViolationType::LargeFile));
    let has_secrets = truffle_stats.total_secrets > 0;

    options.fix_gitignore =
        has_gitignore && prompt_yes_no(input, output, "Add missing .gitignore patterns?")?;
    options.untrack_files = options.fix_gitignore
        && prompt_yes_no(
            input,
            output,
            "Also untrack the affected files while keeping them locally?",
        )?;
    options.fix_large =
        has_large && prompt_yes_no(input, output, "Remove large files from Git history?")?;
    options.fix_secrets =
        has_secrets && prompt_yes_no(input, output, "Remove secrets from Git history?")?;
    Ok(options.fix_gitignore || options.fix_large || options.fix_secrets)
}

fn prompt_yes_no<R: BufRead, W: Write>(
    input: &mut R,
    output: &mut W,
    prompt: &str,
) -> Result<bool> {
    write!(output, "{prompt} [y/N]: ")?;
    output.flush()?;
    let mut response = String::new();
    input.read_line(&mut response)?;
    Ok(matches!(
        response.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// Show summary of fixes to be applied
async fn show_fix_summary(repos: &[FixRepository], options: &FixOptions) -> Result<()> {
    eprintln!("\n📋 Fix Summary\n");

    let mut total_gitignore = 0;
    let mut total_large = 0;
    let mut total_patterns = 0;

    for repo in repos {
        for violation in &repo.violations {
            match violation.violation_type {
                ViolationType::GitignoreViolation => total_gitignore += 1,
                ViolationType::LargeFile => total_large += 1,
                ViolationType::UniversalBadPattern => total_patterns += 1,
            }
        }
    }

    eprintln!("Found selected fixes in {} repositories:", repos.len());

    if options.fix_gitignore {
        eprintln!(
            "  📝 {} files need .gitignore entries",
            total_gitignore + total_patterns
        );
        if options.untrack_files {
            eprintln!("     → Will untrack files after adding to .gitignore");
        } else {
            eprintln!("     → Will only add to .gitignore (files remain tracked)");
        }
    }

    if options.fix_large {
        eprintln!("  📦 {total_large} large files in history");
        eprintln!("     → Will remove from Git history (requires coordinated publication)");
    }

    if options.fix_secrets {
        eprintln!("  🔑 Secrets will be scanned and removed");
        eprintln!("     → Will rewrite Git history (requires coordinated publication)");
    }

    if options.dry_run {
        eprintln!("\n⚠️  DRY RUN MODE - No changes will be made");
    }

    Ok(())
}

/// Prompt user for confirmation
async fn confirm_fixes(options: &FixOptions) -> Result<bool> {
    if options.dry_run {
        return Ok(true); // Always proceed in dry-run mode
    }

    eprintln!("\n═══════════════════════════════════════════════════════════════════");
    eprintln!("⚠️  CONFIRMATION REQUIRED");
    eprintln!("═══════════════════════════════════════════════════════════════════");

    if options.fix_large || options.fix_secrets {
        eprintln!("\n🔴 DESTRUCTIVE OPERATION - HISTORY REWRITE");
        eprintln!("   • Git history will be permanently rewritten");
        eprintln!("   • Full-ref bundles are saved under .git/repos-backups/");
        eprintln!("   • {HISTORY_PUBLICATION_GUIDANCE}");
        eprintln!("   • git-filter-repo normally removes origin; record its reviewed URL now");
        eprintln!("   • All collaborators must re-clone after rewritten refs are published");
        eprintln!("\n   RECOVERY: clone the reported before.bundle into a clean directory");
    } else if options.untrack_files {
        eprintln!("\n🟡 MODERATE OPERATION - FILE UNTRACKING");
        eprintln!("   • Files will be removed from Git tracking");
        eprintln!("   • Files remain in your working directory");
        eprintln!("   • Changes are reversible with: git add <files>");
    } else {
        eprintln!("\n🟢 SAFE OPERATION - GITIGNORE UPDATE");
        eprintln!("   • Only .gitignore files will be modified");
        eprintln!("   • Files remain tracked until manually untracked");
        eprintln!("   • Changes are easily reversible");
    }

    eprintln!("\n═══════════════════════════════════════════════════════════════════");

    eprint!("\nType 'yes' to proceed or anything else to cancel: ");
    io::stderr().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(input.trim().to_lowercase() == "yes")
}

/// Show final results of fix operations
fn show_fix_results(results: &[FixResult]) {
    eprintln!("\n═══════════════════════════════════════════════════════════════════");

    let successful = results.iter().filter(|r| r.errors.is_empty()).count();
    let failed = results.iter().filter(|r| !r.errors.is_empty()).count();

    eprintln!("✅ Fix Summary: {successful} successful, {failed} failed");

    if failed > 0 {
        eprintln!("\n⚠️  Failed fixes:");
        for result in results.iter().filter(|r| !r.errors.is_empty()) {
            eprintln!("  {} - {}", result.repo_name, result.errors.join(", "));
        }
    }

    eprintln!("═══════════════════════════════════════════════════════════════════");
}

#[cfg(test)]
mod tests {
    use super::{select_interactive_fixes_with_io, FixOptions};
    use crate::audit::hygiene::report::HygieneStatus;
    use crate::audit::hygiene::{HygieneStatistics, HygieneViolation, ViolationType};
    use crate::audit::scanner::TruffleStatistics;
    use std::io::Cursor;

    fn interactive_options() -> FixOptions {
        FixOptions {
            interactive: true,
            fix_gitignore: false,
            fix_large: false,
            fix_secrets: false,
            untrack_files: false,
            dry_run: false,
            skip_confirm: false,
        }
    }

    fn available_fixes() -> (HygieneStatistics, TruffleStatistics) {
        let mut hygiene = HygieneStatistics::new();
        hygiene.update(
            "repo",
            "/repo",
            &HygieneStatus::Violations,
            "violations",
            vec![
                HygieneViolation {
                    file_path: "app.log".to_string(),
                    violation_type: ViolationType::UniversalBadPattern,
                    size_bytes: None,
                },
                HygieneViolation {
                    file_path: "large.bin".to_string(),
                    violation_type: ViolationType::LargeFile,
                    size_bytes: Some(2_000_000),
                },
            ],
        );
        let mut truffle = TruffleStatistics::new();
        truffle.total_secrets = 1;
        (hygiene, truffle)
    }

    #[test]
    fn interactive_selection_applies_only_affirmative_answers() {
        let (hygiene, truffle) = available_fixes();
        let mut options = interactive_options();
        let mut input = Cursor::new(b"yes\nno\nyes\nno\n".to_vec());
        let mut output = Vec::new();

        let selected = select_interactive_fixes_with_io(
            &hygiene,
            &truffle,
            &mut options,
            &mut input,
            &mut output,
        )
        .expect("interactive selection");

        assert!(selected);
        assert!(options.fix_gitignore);
        assert!(!options.untrack_files);
        assert!(options.fix_large);
        assert!(!options.fix_secrets);
    }

    #[test]
    fn interactive_eof_and_negative_answers_cancel_safely() {
        let hygiene = HygieneStatistics::new();
        let mut truffle = TruffleStatistics::new();
        truffle.total_secrets = 1;

        for response in [b"".as_slice(), b"no\n".as_slice()] {
            let mut options = interactive_options();
            let mut input = Cursor::new(response.to_vec());
            let mut output = Vec::new();
            let selected = select_interactive_fixes_with_io(
                &hygiene,
                &truffle,
                &mut options,
                &mut input,
                &mut output,
            )
            .expect("safe cancellation");
            assert!(!selected);
            assert!(!options.fix_secrets);
        }
    }
}
