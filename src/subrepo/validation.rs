//! Validation logic for discovering nested repositories

use super::{
    get_commit_timestamp, get_current_commit, get_remote_url, has_uncommitted_changes,
    SubrepoInstance, ValidationReport,
};
use crate::core::config::SKIP_DIRECTORIES;
use anyhow::Result;
use ignore::WalkBuilder;
use std::collections::HashMap;
use std::path::Path;

/// Discover all nested repositories and generate a validation report
pub fn validate_subrepos() -> Result<ValidationReport> {
    let parent_repos = crate::core::discovery::find_repos();
    let mut all_nested = Vec::new();

    println!(
        "🔍 Scanning {} parent repositories for nested repos...\n",
        parent_repos.len()
    );

    for (parent_name, parent_path) in parent_repos {
        let nested = find_nested_in_parent(&parent_name, &parent_path)?;
        all_nested.extend(nested);
    }

    // Group by remote URL
    let mut by_remote: HashMap<String, Vec<SubrepoInstance>> = HashMap::new();
    let mut no_remote = Vec::new();

    for instance in all_nested {
        if let Some(ref remote) = instance.remote_url {
            by_remote.entry(remote.clone()).or_default().push(instance);
        } else {
            no_remote.push(instance);
        }
    }

    let total_nested = by_remote.values().map(|v| v.len()).sum::<usize>() + no_remote.len();

    Ok(ValidationReport {
        total_nested,
        by_remote,
        no_remote,
    })
}

/// Find nested repositories within a parent repository
fn find_nested_in_parent(parent_name: &str, parent_path: &Path) -> Result<Vec<SubrepoInstance>> {
    let mut nested = Vec::new();

    // Walk the parent looking for nested .git directories
    let walker = WalkBuilder::new(parent_path)
        .follow_links(false)
        .max_depth(Some(5)) // Don't go too deep
        .filter_entry(|entry| {
            let file_name = entry.file_name().to_str().unwrap_or("");

            // Skip build/dependency directories
            if SKIP_DIRECTORIES.contains(&file_name) {
                return false;
            }

            // Skip .git directories themselves from walking
            if file_name == ".git" {
                return false;
            }

            true
        })
        .build();

    for entry in walker.flatten() {
        let path = entry.path();

        // Only check directories
        if !entry.file_type().is_some_and(|ft| ft.is_dir()) {
            continue;
        }

        // Skip the parent's root directory
        if path == parent_path {
            continue;
        }

        // Check if this directory has a .git
        let git_path = path.join(".git");
        if !git_path.exists() {
            continue;
        }

        // This is a nested repo! Get its info
        let subrepo_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let relative_path = path
            .strip_prefix(parent_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        // Get git info
        let commit_hash = match get_current_commit(path) {
            Ok(hash) => hash,
            Err(_) => continue, // Skip if can't get commit
        };

        let short_hash = commit_hash.chars().take(7).collect();
        let remote_url = get_remote_url(path).ok();
        let uncommitted = has_uncommitted_changes(path);
        let commit_timestamp = get_commit_timestamp(path, &commit_hash);

        nested.push(SubrepoInstance {
            parent_repo: parent_name.to_string(),
            parent_path: parent_path.to_path_buf(),
            subrepo_name,
            subrepo_path: path.to_path_buf(),
            relative_path,
            commit_hash,
            short_hash,
            remote_url,
            has_uncommitted: uncommitted,
            commit_timestamp,
        });
    }

    Ok(nested)
}

/// Display the validation report
pub fn display_report(report: &ValidationReport) {
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("📊 Validation Report");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("Total nested repos found: {}", report.total_nested);
    println!("Unique remote URLs: {}", report.unique_remotes());
    println!(
        "Shared subrepos (same remote): {}",
        report.shared_subrepos_count()
    );
    println!();

    if report.total_nested == 0 {
        println!("❌ No nested repositories found.");
        println!("   Subrepo tracking feature is NOT needed.");
        return;
    }

    // Show instances by remote
    if !report.by_remote.is_empty() {
        println!("📦 Nested Repositories by Remote:");
        println!();

        for (remote, instances) in &report.by_remote {
            let count = instances.len();
            let name = &instances[0].subrepo_name;

            if count > 1 {
                println!("🔗 {} (found in {} parents)", name, count);
            } else {
                println!("🔗 {} (unique)", name);
            }
            println!("   Remote: {}", remote);

            for instance in instances {
                let uncommitted = if instance.has_uncommitted {
                    " (uncommitted)"
                } else {
                    ""
                };
                println!(
                    "     • {} @ {}{}",
                    instance.parent_repo, instance.short_hash, uncommitted
                );
            }
            println!();
        }
    }

    // Show repos without remotes
    if !report.no_remote.is_empty() {
        println!(
            "⚠️  Nested repos without remotes ({}):",
            report.no_remote.len()
        );
        for instance in &report.no_remote {
            println!("   • {}/{}", instance.parent_repo, instance.subrepo_name);
        }
        println!();
    }

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("💡 Recommendation:");

    let shared_count = report.shared_subrepos_count();

    if shared_count >= 3 {
        println!(
            "   ✅ BUILD IT - You have {} subrepos shared across multiple parents",
            shared_count
        );
        println!("      This feature would help track drift between them.");
    } else if shared_count > 0 {
        println!("   ⚠️  MAYBE - You have {} shared subrepos", shared_count);
        println!("      Consider if manual tracking is sufficient.");
    } else {
        println!("   ❌ SKIP IT - All nested repos are unique (no drift possible)");
        println!("      Subrepo drift tracking is not needed.");
    }

    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
}
