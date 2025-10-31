//! Repository discovery and initialization utilities

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use super::config::{DEFAULT_REPO_NAME, SKIP_DIRECTORIES, UNKNOWN_REPO_NAME};

/// Check if a .git file (for submodules/worktrees) contains gitdir reference
/// Only reads the first 512 bytes for efficiency
fn is_git_file(path: &Path) -> bool {
    match fs::File::open(path) {
        Ok(file) => {
            let reader = BufReader::new(file);
            // Only read first few lines - gitdir is typically in the first line
            reader
                .lines()
                .take(5)
                .filter_map(Result::ok)
                .any(|line| line.trim_start().starts_with("gitdir:"))
        }
        Err(_) => false,
    }
}

/// Recursively searches for git repositories in the current directory
/// Returns a vector of (repository_name, path) tuples with deduplication
pub fn find_repos() -> Vec<(String, PathBuf)> {
    let mut repositories = Vec::new();
    let mut seen_paths = HashSet::new();
    let mut name_counts = HashMap::new();

    // Walk through directory tree, skipping common build/dependency directories
    // and git repository contents
    for entry in WalkDir::new(".")
        .follow_links(true) // Follow symlinks to find symlinked repos (loop detection built-in)
        .into_iter()
        .filter_entry(|e| {
            let file_name = e.file_name().to_str().unwrap_or("");

            // Skip common build/dependency directories
            if SKIP_DIRECTORIES.contains(&file_name) {
                return false;
            }

            // Skip .git directories themselves (don't descend into them)
            // This prevents scanning thousands of files in .git/objects/, etc.
            // We still find nested repos because we continue walking into repo directories
            if file_name == ".git" {
                return false;
            }

            true
        })
        .flatten()
    {
        let path = entry.path();

        // Look for .git directories to identify repositories
        // Check if this directory contains a .git entry
        if entry.file_type().is_dir() {
            let git_path = path.join(".git");

            if git_path.exists() {
                let is_git_repo = if git_path.is_dir() {
                    true
                } else if git_path.is_file() {
                    // Submodules and worktrees expose a .git file
                    is_git_file(&git_path)
                } else {
                    false
                };

                if is_git_repo {
                    // Skip if we've already seen this exact path
                    if !seen_paths.insert(path.to_path_buf()) {
                        continue;
                    }

                    let base_name = if path == Path::new(".") {
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
                        path.file_name()
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

                    repositories.push((repo_name, path.to_path_buf()));
                }
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
