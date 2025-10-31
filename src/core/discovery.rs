//! Repository discovery and initialization utilities

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use ignore::WalkBuilder;
use rayon::prelude::*;

use super::config::{DEFAULT_REPO_NAME, SKIP_DIRECTORIES, UNKNOWN_REPO_NAME, MAX_SCAN_DEPTH, ESTIMATED_REPO_COUNT};

/// Check if a .git file (for submodules/worktrees) contains gitdir reference
/// Only reads the first 5 lines for efficiency
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
    // Pre-allocate collections based on estimated repository count
    let repositories = Arc::new(Mutex::new(Vec::with_capacity(ESTIMATED_REPO_COUNT)));
    let seen_paths = Arc::new(Mutex::new(HashSet::with_capacity(ESTIMATED_REPO_COUNT)));
    let name_counts = Arc::new(Mutex::new(HashMap::with_capacity(ESTIMATED_REPO_COUNT)));

    // Build walker with optimizations
    let walker = WalkBuilder::new(".")
        .follow_links(true) // Follow symlinks to find symlinked repos
        .max_depth(Some(MAX_SCAN_DEPTH)) // Limit depth to avoid deep recursion
        .filter_entry(|entry| {
            let file_name = entry.file_name().to_str().unwrap_or("");

            // Skip common build/dependency directories
            if SKIP_DIRECTORIES.contains(&file_name) {
                return false;
            }

            // Skip .git directories themselves (don't descend into them)
            // This prevents scanning thousands of files in .git/objects/, etc.
            if file_name == ".git" {
                return false;
            }

            true
        })
        .build();

    // Walk the directory tree
    for result in walker {
        if let Ok(entry) = result {
            let path = entry.path();

            // Only check directories
            if !entry.file_type().map_or(false, |ft| ft.is_dir()) {
                continue;
            }

            // Check if this directory contains a .git entry
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
                    {
                        let mut seen = seen_paths.lock().unwrap();
                        if !seen.insert(path.to_path_buf()) {
                            continue;
                        }
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
                    let repo_name = {
                        let mut counts = name_counts.lock().unwrap();
                        let count = counts.entry(base_name.clone()).or_insert(0);
                        *count += 1;
                        if *count > 1 {
                            format!("{}-{}", base_name, count)
                        } else {
                            base_name
                        }
                    };

                    repositories.lock().unwrap().push((repo_name, path.to_path_buf()));
                }
            }
        }
    }

    // Extract repositories from Arc<Mutex<>>
    let mut repos = Arc::try_unwrap(repositories)
        .unwrap_or_else(|arc| (*arc.lock().unwrap()).clone());

    // Sort repositories alphabetically by name (case-insensitive) using parallel sort
    repos.par_sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

    repos
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
