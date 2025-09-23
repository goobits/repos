//! Repository discovery and initialization utilities

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::config::{DEFAULT_REPO_NAME, SKIP_DIRECTORIES, UNKNOWN_REPO_NAME};

/// Recursively searches for git repositories in the current directory
/// Returns a vector of (repository_name, path) tuples with deduplication
pub fn find_repos() -> Vec<(String, PathBuf)> {
    let mut repositories = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut name_counts = HashMap::new();

    // Walk through directory tree, skipping common build/dependency directories
    for entry in WalkDir::new(".")
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| {
            if let Some(file_name) = e.file_name().to_str() {
                !SKIP_DIRECTORIES.contains(&file_name)
            } else {
                true
            }
        })
        .flatten()
    {
        // Look for .git directories to identify repositories
        if entry.file_name() == ".git" {
            let entry_type = entry.file_type();

            let is_git_repo = if entry_type.is_dir() {
                true
            } else if entry_type.is_file() {
                // Submodules and worktrees expose a .git file that points to the actual gitdir.
                match fs::read_to_string(entry.path()) {
                    Ok(content) => content
                        .lines()
                        .any(|line| line.trim_start().starts_with("gitdir:")),
                    Err(_) => false,
                }
            } else {
                false
            };

            if !is_git_repo {
                continue;
            }

            if let Some(parent) = entry.path().parent() {
                // Skip if we've already seen this exact path
                // This treats symlinks as separate repositories per user request
                if !seen_paths.insert(parent.to_path_buf()) {
                    continue;
                }

                let base_name = if parent == Path::new(".") {
                    // If we're in the current directory, use the directory name
                    if let Ok(current_dir) = std::env::current_dir() {
                        current_dir
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(DEFAULT_REPO_NAME)
                            .to_string()
                    } else {
                        DEFAULT_REPO_NAME.to_string()
                    }
                } else {
                    parent
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(UNKNOWN_REPO_NAME)
                        .to_string()
                };

                // Handle duplicate names by adding a suffix
                let count = name_counts.entry(base_name.clone()).or_insert(0);
                *count += 1;
                let repo_name = if *count > 1 {
                    format!("{}-{}", base_name, count)
                } else {
                    base_name
                };

                repositories.push((repo_name, parent.to_path_buf()));
            }
        }
    }

    // Sort repositories alphabetically by name (case-insensitive)
    repositories.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

    repositories
}

/// Common initialization for commands that scan repositories
pub fn init_command(scanning_msg: &str) -> (std::time::Instant, Vec<(String, PathBuf)>) {
    println!();
    print!("{}", scanning_msg);
    std::io::stdout().flush().expect("Failed to flush stdout during repository scanning - this indicates a terminal or I/O issue");

    let start_time = std::time::Instant::now();
    let repos = find_repos();

    (start_time, repos)
}
