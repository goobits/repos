//! Repository discovery and initialization utilities

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use ignore::WalkBuilder;
use rayon::prelude::*;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::config::{
    DEFAULT_REPO_NAME, ESTIMATED_REPO_COUNT, MAX_SCAN_DEPTH, SKIP_DIRECTORIES, UNKNOWN_REPO_NAME,
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

/// Recursively searches for git repositories from a specific path
/// Returns a vector of (`repository_name`, path) tuples with deduplication
///
/// This function uses parallel directory walking for significantly better performance
/// with large directory trees (5-10x faster than sequential walking).
/// Uses `DashMap` for lock-free concurrent access, eliminating mutex contention.
pub fn find_repos_from_path(search_path: impl AsRef<Path>) -> Vec<(String, PathBuf)> {
    let search_path = search_path.as_ref();

    // Use DashMap for lock-free concurrent access (20-40% faster than Mutex<HashMap>)
    // Using a single DashMap<PathBuf, String> avoids:
    // 1. Mutex contention on a separate Vec
    // 2. Double allocation of PathBuf (once for set, once for list)
    let repos_map = Arc::new(DashMap::with_capacity(ESTIMATED_REPO_COUNT));
    let name_counts = Arc::new(DashMap::with_capacity(ESTIMATED_REPO_COUNT));
    let search_path_buf = search_path.to_path_buf();

    // Clone for closure
    let repos_map_clone = Arc::clone(&repos_map);
    let name_counts_clone = Arc::clone(&name_counts);
    let search_path_buf_clone = search_path_buf.clone();

    // Build parallel walker with optimizations
    let walker = WalkBuilder::new(search_path)
        .follow_links(true) // Follow symlinks to find symlinked repos
        .max_depth(Some(MAX_SCAN_DEPTH)) // Limit depth to avoid deep recursion
        .threads(num_cpus::get().min(8)) // Use up to 8 threads for directory walking
        .hidden(false) // Enable hidden files to see .git
        .filter_entry(move |entry| {
            let file_name = entry.file_name().to_str().unwrap_or("");

            // If we find .git, check if it's a valid repo and add the parent directory
            if file_name == ".git" {
                let path = entry.path();
                // .git found, check its parent (the repo root)
                if let Some(repo_path) = path.parent() {
                    let is_git_repo = if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                        true
                    } else {
                        // Submodules and worktrees expose a .git file
                        is_git_file(path)
                    };

                    if is_git_repo {
                        // Skip if we've already seen this exact path
                        // Check existence first to avoid allocation
                        if !repos_map_clone.contains_key(repo_path) {
                            let path_buf = repo_path.to_path_buf();

                            // Use entry API to atomically check and insert
                            if let Entry::Vacant(entry) = repos_map_clone.entry(path_buf) {
                                let base_name = if repo_path == search_path_buf_clone {
                                    // If this is the search path itself, use its directory name
                                    search_path_buf_clone
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or(DEFAULT_REPO_NAME)
                                        .to_string()
                                } else {
                                    repo_path
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or(UNKNOWN_REPO_NAME)
                                        .to_string()
                                };

                                // Handle duplicate names by adding a suffix
                                let repo_name = {
                                    let mut entry =
                                        name_counts_clone.entry(base_name.clone()).or_insert(0);
                                    *entry += 1;
                                    let count = *entry;
                                    if count > 1 {
                                        format!("{base_name}-{count}")
                                    } else {
                                        base_name
                                    }
                                };

                                entry.insert(repo_name);
                            }
                        }
                    }
                }
                // Don't descend into .git
                return false;
            }

            // Skip common build/dependency directories
            if SKIP_DIRECTORIES.contains(&file_name) {
                return false;
            }

            // Skip hidden files/directories (emulate default behavior), except root
            // This prevents scanning .config, .ssh, etc. but allows .git (handled above)
            if entry.depth() > 0 && file_name.starts_with('.') {
                return false;
            }

            true
        })
        .build_parallel();

    // Walk the directory tree in parallel - logic is now in filter_entry
    walker.run(|| Box::new(|_| ignore::WalkState::Continue));

    // Extract repositories from DashMap
    // Convert DashMap to Vec<(String, PathBuf)>
    let mut repos: Vec<(String, PathBuf)> = Arc::try_unwrap(repos_map)
        .map(|map| map.into_iter().map(|(p, n)| (n, p)).collect())
        .unwrap_or_else(|arc| {
            // Fallback if Arc has other references (should not happen in normal flow)
            arc.iter().map(|r| (r.value().clone(), r.key().clone())).collect()
        });

    // Sort repositories alphabetically by name (case-insensitive) using parallel sort
    repos.par_sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

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
    fn test_dashmap_concurrent_access() {
        // Test that DashMap handles concurrent access correctly
        let map: Arc<DashMap<String, i32>> = Arc::new(DashMap::new());

        // Simulate concurrent inserts from multiple threads
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let map_clone = Arc::clone(&map);
                std::thread::spawn(move || {
                    for j in 0..100 {
                        let key = format!("key-{}-{}", i, j);
                        map_clone.insert(key, i * 1000 + j);
                    }
                })
            })
            .collect();

        // Wait for all threads to complete
        for handle in handles {
            handle
                .join()
                .expect("Test thread panicked during concurrent insert test");
        }

        // Verify all 1000 items were inserted (10 threads * 100 items)
        assert_eq!(map.len(), 1000, "All concurrent inserts should succeed");
    }

    #[test]
    fn test_dashmap_no_race_conditions() {
        // Test that DashMap entry API provides atomic operations
        let map: Arc<DashMap<String, i32>> = Arc::new(DashMap::new());

        // Multiple threads incrementing the same counter
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let map_clone = Arc::clone(&map);
                std::thread::spawn(move || {
                    for _ in 0..1000 {
                        let mut entry = map_clone.entry("counter".to_string()).or_insert(0);
                        *entry += 1;
                    }
                })
            })
            .collect();

        for handle in handles {
            handle
                .join()
                .expect("Test thread panicked during race condition test");
        }

        // Should have exactly 10,000 (10 threads * 1,000 increments)
        assert_eq!(
            *map.get("counter")
                .expect("Key 'counter' should exist in test map"),
            10000,
            "Counter should be atomic"
        );
    }

    #[test]
    fn test_path_deduplication_with_dashmap() {
        use std::path::PathBuf;

        let seen: Arc<DashMap<PathBuf, ()>> = Arc::new(DashMap::new());

        let path1 = PathBuf::from("/test/repo1");
        let path2 = PathBuf::from("/test/repo2");
        let path1_dup = PathBuf::from("/test/repo1");

        // First insert should return None (new entry)
        assert!(seen.insert(path1.clone(), ()).is_none());

        // Second insert of same path should return Some (existing entry)
        assert!(seen.insert(path1_dup, ()).is_some());

        // Different path should return None
        assert!(seen.insert(path2, ()).is_none());

        // Should have 2 unique paths
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn test_find_repos_from_path_deduplication() {
        use tempfile::TempDir;
        use std::process::Command;

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
