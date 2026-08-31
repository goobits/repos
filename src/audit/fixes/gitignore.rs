//! Safe `.gitignore` updates and optional tracked-file removal.

use super::{ensure_command_success, FixOptions};
use crate::audit::hygiene::{HygieneViolation, ViolationType};
use anyhow::Result;
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::Path;
use tokio::process::Command;

pub(super) async fn fix_gitignore_violations(
    repo_path: &Path,
    violations: &[HygieneViolation],
    options: &FixOptions,
) -> Result<String> {
    let gitignore_violations: Vec<_> = violations
        .iter()
        .filter(|violation| {
            matches!(
                violation.violation_type,
                ViolationType::GitignoreViolation | ViolationType::UniversalBadPattern
            )
        })
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

    let patterns = group_gitignore_patterns(&gitignore_violations);
    let gitignore_path = Path::new(repo_path).join(".gitignore");
    let existing_content = fs::read_to_string(&gitignore_path).unwrap_or_default();
    let existing_patterns: HashSet<_> = existing_content
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .collect();

    let new_patterns = patterns
        .into_iter()
        .filter(|pattern| !existing_patterns.contains(pattern.as_str()))
        .collect::<Vec<_>>();

    if !new_patterns.is_empty() {
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
    }

    let mut untracked_count = 0;
    if options.untrack_files {
        let paths = gitignore_violations
            .iter()
            .map(|violation| violation.file_path.as_str())
            .collect::<BTreeSet<_>>();
        if !paths.is_empty() {
            let mut command = Command::new("git");
            command.args(["rm", "--cached", "-r", "--ignore-unmatch", "--"]);
            for path in &paths {
                command.arg(format!(":(literal){path}"));
            }
            let result = command.current_dir(repo_path).output().await?;
            ensure_command_success(&result, "git rm --cached")?;
            untracked_count = paths.len();
        }
    }

    if new_patterns.is_empty() && untracked_count == 0 {
        return Ok("All patterns already in .gitignore".to_string());
    }

    if !new_patterns.is_empty() {
        let add_output = Command::new("git")
            .args(["add", "--", ".gitignore"])
            .current_dir(repo_path)
            .output()
            .await?;
        ensure_command_success(&add_output, "staging .gitignore")?;
    }

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

    let commit_output = Command::new("git")
        .args(["commit", "-m", &commit_message])
        .current_dir(repo_path)
        .output()
        .await?;
    ensure_command_success(&commit_output, "committing .gitignore fixes")?;

    Ok(format!(
        "Added {} patterns to .gitignore{}",
        new_patterns.len(),
        if untracked_count > 0 {
            format!(", untracked {untracked_count} files")
        } else {
            String::new()
        }
    ))
}

fn group_gitignore_patterns(violations: &[&HygieneViolation]) -> Vec<String> {
    let mut patterns = BTreeSet::new();

    for violation in violations {
        let path = &violation.file_path;

        if path.contains("node_modules/") {
            patterns.insert("node_modules/");
        } else if path.contains("target/debug/") {
            patterns.insert("target/debug/");
        } else if path.contains("target/release/") {
            patterns.insert("target/release/");
        } else if path.contains("dist/") {
            patterns.insert("dist/");
        } else if path.contains("build/") {
            patterns.insert("build/");
        } else if path.contains("__pycache__/") {
            patterns.insert("__pycache__/");
        } else if path.contains(".venv/") {
            patterns.insert(".venv/");
        } else if path.ends_with(".log") {
            patterns.insert("*.log");
        } else if path.ends_with(".tmp") {
            patterns.insert("*.tmp");
        } else if path.ends_with(".cache") {
            patterns.insert("*.cache");
        } else if path == ".DS_Store" {
            patterns.insert(".DS_Store");
        } else if path == "Thumbs.db" {
            patterns.insert("Thumbs.db");
        } else if path == ".env" {
            patterns.insert(".env");
        } else if path.ends_with(".key") || path.ends_with(".pem") {
            patterns.insert("*.key");
            patterns.insert("*.pem");
        } else {
            patterns.insert(path.as_str());
        }
    }

    patterns.into_iter().map(str::to_string).collect()
}
