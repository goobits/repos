//! Integration tests for repository discovery functionality

mod common;

use common::{create_multiple_repos, is_git_available, setup_git_repo, TestRepoBuilder};
use goobits_repos::core::find_repos_from_path;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_find_single_repo() {
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let repo = TestRepoBuilder::new("test-repo")
        .build()
        .expect("Failed to create test repo");

    // Move repo to a known location
    let repos_dir = temp_dir.path().join("repos");
    fs::create_dir(&repos_dir).expect("Failed to create repos directory");
    let repo_path = repos_dir.join("my-repo");
    fs::rename(repo.path(), &repo_path).expect("Failed to move repo");

    // Find repositories from the repos directory
    let found_repos = find_repos_from_path(&repos_dir);

    assert_eq!(found_repos.len(), 1, "Should find exactly one repository");
    assert_eq!(found_repos[0].0, "my-repo");
}

#[test]
fn test_find_multiple_repos() {
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let temp_dir = TempDir::new().expect("Failed to create temp directory");

    // Create 5 test repositories
    create_multiple_repos(temp_dir.path(), 5).expect("Failed to create repos");

    // Find repositories from the temp directory
    let found_repos = find_repos_from_path(temp_dir.path());

    assert_eq!(found_repos.len(), 5, "Should find all 5 repositories");

    // Verify all repos were found (they should be sorted alphabetically)
    let repo_names: Vec<_> = found_repos.iter().map(|(name, _)| name.as_str()).collect();
    assert!(repo_names.contains(&"test-repo-1"));
    assert!(repo_names.contains(&"test-repo-2"));
    assert!(repo_names.contains(&"test-repo-3"));
    assert!(repo_names.contains(&"test-repo-4"));
    assert!(repo_names.contains(&"test-repo-5"));
}

#[test]
fn test_find_repos_with_duplicate_names() {
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let temp_dir = TempDir::new().expect("Failed to create temp directory");

    // Create repos with the same name in different directories
    let dir1 = temp_dir.path().join("project1");
    let dir2 = temp_dir.path().join("project2");
    fs::create_dir(&dir1).expect("Failed to create dir1");
    fs::create_dir(&dir2).expect("Failed to create dir2");

    let repo1 = dir1.join("my-app");
    let repo2 = dir2.join("my-app");
    fs::create_dir(&repo1).expect("Failed to create repo1");
    fs::create_dir(&repo2).expect("Failed to create repo2");

    setup_git_repo(&repo1).expect("Failed to setup repo1");
    setup_git_repo(&repo2).expect("Failed to setup repo2");

    // Find repositories from the temp directory
    let found_repos = find_repos_from_path(temp_dir.path());

    assert_eq!(found_repos.len(), 2, "Should find both repositories");

    // Verify that duplicate names have suffixes
    let repo_names: Vec<_> = found_repos.iter().map(|(name, _)| name.as_str()).collect();
    assert!(
        repo_names.contains(&"my-app") && repo_names.contains(&"my-app-2"),
        "Should have 'my-app' and 'my-app-2', got: {:?}",
        repo_names
    );
}

#[test]
fn test_skips_node_modules() {
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let temp_dir = TempDir::new().expect("Failed to create temp directory");

    // Create a valid repo
    create_multiple_repos(temp_dir.path(), 1).expect("Failed to create repo");

    // Create a repo inside node_modules (should be skipped)
    let node_modules = temp_dir.path().join("node_modules");
    fs::create_dir(&node_modules).expect("Failed to create node_modules");
    let nested_repo = node_modules.join("some-package");
    fs::create_dir(&nested_repo).expect("Failed to create nested repo");
    setup_git_repo(&nested_repo).expect("Failed to setup nested repo");

    // Find repositories from the temp directory
    let found_repos = find_repos_from_path(temp_dir.path());

    // Should only find the valid repo, not the one in node_modules
    assert_eq!(found_repos.len(), 1, "Should skip repo in node_modules");
    assert_eq!(found_repos[0].0, "test-repo-1");
}

#[test]
fn test_max_depth_limit() {
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let temp_dir = TempDir::new().expect("Failed to create temp directory");

    // Create deeply nested directory structure (depth 12)
    let mut current_path = temp_dir.path().to_path_buf();
    for i in 1..=12 {
        current_path = current_path.join(format!("level{}", i));
        fs::create_dir(&current_path).expect("Failed to create directory");
    }

    // Create a repo at depth 12 (should be beyond MAX_SCAN_DEPTH of 10)
    setup_git_repo(&current_path).expect("Failed to setup deep repo");

    // Create a repo at depth 2 (should be found)
    let shallow_repo = temp_dir.path().join("level1").join("shallow-repo");
    fs::create_dir(&shallow_repo).expect("Failed to create shallow repo");
    setup_git_repo(&shallow_repo).expect("Failed to setup shallow repo");

    // Find repositories from the temp directory
    let found_repos = find_repos_from_path(temp_dir.path());

    // Should only find the shallow repo, not the deep one
    assert_eq!(
        found_repos.len(),
        1,
        "Should not find repo beyond max depth"
    );
    assert_eq!(found_repos[0].0, "shallow-repo");
}

#[test]
fn test_handles_symlinks() {
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let temp_dir = TempDir::new().expect("Failed to create temp directory");

    // Create a real repo
    let real_repo = temp_dir.path().join("real-repo");
    fs::create_dir(&real_repo).expect("Failed to create real repo");
    setup_git_repo(&real_repo).expect("Failed to setup real repo");

    // Create a symlink to it
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let symlink_path = temp_dir.path().join("symlink-repo");
        if symlink(&real_repo, &symlink_path).is_ok() {
            // Find repositories from the temp directory
            let found_repos = find_repos_from_path(temp_dir.path());

            // Should find both the real repo and symlink (with deduplication)
            // Depending on implementation, might find 1 or 2
            assert!(
                !found_repos.is_empty() && found_repos.len() <= 2,
                "Should handle symlinks correctly"
            );
        }
    }
}

#[test]
fn test_current_directory_as_repo() {
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    setup_git_repo(temp_dir.path()).expect("Failed to setup repo");

    // Find repositories - should find the directory itself as a repo
    let found_repos = find_repos_from_path(temp_dir.path());

    // Should find current directory as a repo with appropriate name
    assert_eq!(
        found_repos.len(),
        1,
        "Should find current directory as repo"
    );
    // The name will be derived from the temp directory name
    assert!(
        !found_repos[0].0.is_empty(),
        "Repo name should not be empty"
    );
}

#[test]
fn test_alphabetical_sorting() {
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let temp_dir = TempDir::new().expect("Failed to create temp directory");

    // Create repos with names that will test sorting
    let names = vec!["zebra", "apple", "Banana", "cherry", "DELTA"];
    for name in &names {
        let repo_path = temp_dir.path().join(name);
        fs::create_dir(&repo_path).expect("Failed to create repo dir");
        setup_git_repo(&repo_path).expect("Failed to setup repo");
    }

    // Find repositories from the temp directory
    let found_repos = find_repos_from_path(temp_dir.path());

    assert_eq!(found_repos.len(), 5);

    // Verify alphabetical order (case-insensitive)
    let repo_names: Vec<_> = found_repos.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(
        repo_names,
        vec!["apple", "Banana", "cherry", "DELTA", "zebra"]
    );
}
