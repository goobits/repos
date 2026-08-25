//! Repository discovery and initialization utilities

use dashmap::DashMap;
use ignore::WalkBuilder;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::config::{
    DEFAULT_REPO_NAME, ESTIMATED_REPO_COUNT, FLEET_IGNORE_FILENAME, SKIP_DIRECTORIES,
    UNKNOWN_REPO_NAME,
};

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

fn compare_repository_aliases(left: &Path, right: &Path) -> std::cmp::Ordering {
    let left_is_symlink =
        fs::symlink_metadata(left).is_ok_and(|metadata| metadata.file_type().is_symlink());
    let right_is_symlink =
        fs::symlink_metadata(right).is_ok_and(|metadata| metadata.file_type().is_symlink());

    left_is_symlink
        .cmp(&right_is_symlink)
        .then_with(|| left.components().count().cmp(&right.components().count()))
        .then_with(|| left.cmp(right))
}

/// Recursively searches for git repositories from a specific path
/// Returns a vector of (`repository_name`, path) tuples with deduplication
///
/// Directory walking is parallel, while naming happens after paths are sorted so
/// duplicate-name suffixes are stable across runs.
pub fn find_repos_from_path(search_path: impl AsRef<Path>) -> Vec<(String, PathBuf)> {
    let search_path = search_path.as_ref();

    let repos_seen = Arc::new(DashMap::with_capacity(ESTIMATED_REPO_COUNT));

    // Repository inventory is defined by the filesystem, not by a containing
    // repository's ignore rules. A nested repository is commonly ignored by
    // its parent, but it still needs to participate in fleet operations.
    let walker = WalkBuilder::new(search_path)
        .parents(false)
        .ignore(false)
        .git_ignore(false)
        .git_global(false)
        .git_exclude(false)
        .add_custom_ignore_filename(FLEET_IGNORE_FILENAME)
        .follow_links(true) // Follow symlinks to find symlinked repos
        .threads(
            std::thread::available_parallelism()
                .map(std::num::NonZeroUsize::get)
                .unwrap_or(1)
                .min(8),
        )
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
        .build_parallel();

    // Walk the directory tree in parallel
    walker.run(|| {
        let repos_seen = Arc::clone(&repos_seen);

        Box::new(move |result| {
            use ignore::WalkState;

            if let Ok(entry) = result {
                let path = entry.path();

                // Only check directories
                if !entry.file_type().is_some_and(|ft| ft.is_dir()) {
                    return WalkState::Continue;
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
                        repos_seen.insert(path.to_path_buf(), ());
                    }
                }
            }

            WalkState::Continue
        })
    });

    let paths: Vec<PathBuf> = Arc::try_unwrap(repos_seen)
        .map(|map| map.into_iter().map(|(path, ())| path).collect())
        .unwrap_or_else(|arc| arc.iter().map(|entry| entry.key().clone()).collect());
    let mut physical_repositories = HashMap::<PathBuf, PathBuf>::with_capacity(paths.len());
    for path in paths {
        let physical_path = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        physical_repositories
            .entry(physical_path)
            .and_modify(|representative| {
                if compare_repository_aliases(&path, representative).is_lt() {
                    *representative = path.clone();
                }
            })
            .or_insert(path);
    }
    let mut paths = physical_repositories.into_values().collect::<Vec<_>>();
    paths.sort();

    let mut name_counts = HashMap::with_capacity(paths.len());
    let mut repos: Vec<(String, PathBuf)> = paths
        .into_iter()
        .map(|path| {
            let base_name = if path == search_path {
                search_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(DEFAULT_REPO_NAME)
            } else {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(UNKNOWN_REPO_NAME)
            };
            let count = name_counts.entry(base_name.to_string()).or_insert(0);
            *count += 1;
            let name = if *count == 1 {
                base_name.to_string()
            } else {
                format!("{base_name}-{count}")
            };
            (name, path)
        })
        .collect();

    repos.sort_by(|a, b| {
        a.0.to_lowercase()
            .cmp(&b.0.to_lowercase())
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.1.cmp(&b.1))
    });

    repos
}

/// Recursively searches for git repositories in the current directory
/// Returns a vector of (`repository_name`, path) tuples with deduplication
///
/// This is a convenience wrapper around `find_repos_from_path()` that searches
/// from the current working directory.
pub fn find_repos() -> Vec<(String, PathBuf)> {
    find_repos_from_path(".")
}

/// Common initialization for commands that scan repositories
#[must_use]
pub async fn init_command(scanning_msg: &str) -> (std::time::Instant, Vec<(String, PathBuf)>) {
    println!();
    print!("{scanning_msg}");
    // Flush stdout - ignore errors as this is non-critical
    let _ = std::io::stdout().flush();

    init_command_quiet().await
}

/// Common initialization without a human-oriented scanning preamble.
#[must_use]
pub(crate) async fn init_command_quiet() -> (std::time::Instant, Vec<(String, PathBuf)>) {
    let start_time = std::time::Instant::now();
    let repos = tokio::task::spawn_blocking(find_repos)
        .await
        .unwrap_or_else(|e| {
            eprintln!("Error in repository discovery: {e}");
            Vec::new()
        });

    (start_time, repos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_repos_from_path_deduplication() {
        use std::process::Command;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        // Create 3 repos
        let repo1 = root.join("repo1");
        let repo2 = root.join("repo2");
        let repo3 = root.join("repo3");

        for path in [&repo1, &repo2, &repo3] {
            fs::create_dir(path).unwrap();
            Command::new("git")
                .arg("init")
                .arg("-q")
                .current_dir(path)
                .output()
                .unwrap();
        }

        // Run discovery
        let repos = find_repos_from_path(root);

        assert_eq!(repos.len(), 3);

        // Check names
        let names: Vec<_> = repos.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"repo1"));
        assert!(names.contains(&"repo2"));
        assert!(names.contains(&"repo3"));
    }
}
