use anyhow::Result;
use goobits_repos::subrepo::{sync::sync_subrepo_with_report, SubrepoInstance, ValidationReport};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

mod common;
use common::git::{create_test_commit, setup_git_repo};

fn clone_repo(source: &Path, dest: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(["clone", source.to_str().unwrap(), dest.to_str().unwrap()])
        .output()?;

    if !output.status.success() {
        anyhow::bail!("Failed to clone repo");
    }
    Ok(())
}

fn get_head_commit(path: &Path) -> Result<String> {
    let output = Command::new("git")
        .args(["-C", path.to_str().unwrap(), "rev-parse", "HEAD"])
        .output()?;

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

#[test]
fn test_sync_subrepo_success() -> Result<()> {
    // 1. Setup
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();

    // Create "remote" repo (the shared subrepo source)
    let remote_path = root.join("upstream-lib");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "lib.rs", "fn hello() {}", "Initial commit")?;

    // Get initial commit
    let initial_commit = get_head_commit(&remote_path)?;

    // Create a newer commit in remote
    create_test_commit(
        &remote_path,
        "lib.rs",
        "fn hello() { println!(); }",
        "Update",
    )?;
    let target_commit = get_head_commit(&remote_path)?;

    // Create Parent Repo A with subrepo at "lib"
    let parent_a_path = root.join("app-a");
    std::fs::create_dir(&parent_a_path)?;
    setup_git_repo(&parent_a_path)?;
    let subrepo_a_path = parent_a_path.join("lib");
    clone_repo(&remote_path, &subrepo_a_path)?;
    // Checkout initial commit in subrepo A (simulate old state)
    Command::new("git")
        .args([
            "-C",
            subrepo_a_path.to_str().unwrap(),
            "checkout",
            &initial_commit,
        ])
        .output()?;

    // Create Parent Repo B with subrepo at "libs/mylib"
    let parent_b_path = root.join("app-b");
    std::fs::create_dir(&parent_b_path)?;
    setup_git_repo(&parent_b_path)?;
    let subrepo_b_path = parent_b_path.join("libs/mylib");
    std::fs::create_dir_all(subrepo_b_path.parent().unwrap())?;
    clone_repo(&remote_path, &subrepo_b_path)?;
    // Checkout initial commit in subrepo B
    Command::new("git")
        .args([
            "-C",
            subrepo_b_path.to_str().unwrap(),
            "checkout",
            &initial_commit,
        ])
        .output()?;

    // 2. Construct Report
    let remote_url = remote_path.to_str().unwrap().to_string(); // Local path as URL
                                                                // Normalize logic in code lowercases it and removes .git, let's just use what we have,
                                                                // but the matching logic in sync uses the report structure.

    let instance_a = SubrepoInstance {
        parent_repo: "app-a".to_string(),
        parent_path: parent_a_path.clone(),
        subrepo_name: "upstream-lib".to_string(), // Name is usually folder name or derived
        subrepo_path: subrepo_a_path.clone(),
        relative_path: "lib".to_string(),
        commit_hash: initial_commit.clone(),
        short_hash: initial_commit[..7].to_string(),
        remote_url: Some(remote_url.clone()),
        has_uncommitted: false,
        commit_timestamp: 0,
    };

    let instance_b = SubrepoInstance {
        parent_repo: "app-b".to_string(),
        parent_path: parent_b_path.clone(),
        subrepo_name: "upstream-lib".to_string(),
        subrepo_path: subrepo_b_path.clone(),
        relative_path: "libs/mylib".to_string(),
        commit_hash: initial_commit.clone(),
        short_hash: initial_commit[..7].to_string(),
        remote_url: Some(remote_url.clone()),
        has_uncommitted: false,
        commit_timestamp: 0,
    };

    let mut by_remote = HashMap::new();
    by_remote.insert(remote_url.clone(), vec![instance_a, instance_b]);

    let report = ValidationReport {
        total_nested: 2,
        by_remote,
        no_remote: vec![],
    };

    // 3. Run Sync
    sync_subrepo_with_report("upstream-lib", &target_commit, false, false, &report)?;

    // 4. Verify
    let head_a = get_head_commit(&subrepo_a_path)?;
    let head_b = get_head_commit(&subrepo_b_path)?;

    assert_eq!(head_a, target_commit, "Repo A should be synced to target");
    assert_eq!(head_b, target_commit, "Repo B should be synced to target");

    Ok(())
}
