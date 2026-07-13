use anyhow::Result;
use goobits_repos::subrepo::{
    sync::sync_subrepo_with_report, sync::update_subrepo_with_report, SubrepoInstance,
    ValidationReport,
};
use std::collections::HashMap;
use std::process::Command;
use tempfile::TempDir;

mod common;
use common::git::{clone_repo, create_test_commit, get_head_commit, setup_git_repo};

#[test]
fn test_sync_with_uncommitted_changes_stash() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();

    // 1. Upstream
    let remote_path = root.join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "f.txt", "v1", "Initial")?;
    let commit1 = get_head_commit(&remote_path)?;
    create_test_commit(&remote_path, "f.txt", "v2", "Update")?;
    let commit2 = get_head_commit(&remote_path)?;

    // 2. Parent with subrepo
    let parent_path = root.join("parent");
    std::fs::create_dir(&parent_path)?;
    setup_git_repo(&parent_path)?;
    let sub_path = parent_path.join("sub");
    clone_repo(&remote_path, &sub_path)?;
    Command::new("git")
        .args(["-C", sub_path.to_str().unwrap(), "checkout", &commit1])
        .output()?;

    // 3. Create uncommitted change in subrepo
    std::fs::write(sub_path.join("dirty.txt"), "mod")?;

    // 4. Report
    let instance = SubrepoInstance {
        parent_repo: "parent".to_string(),
        parent_path: parent_path.clone(),
        subrepo_name: "upstream".to_string(),
        subrepo_path: sub_path.clone(),
        relative_path: "sub".to_string(),
        commit_hash: commit1.clone(),
        short_hash: commit1[..7].to_string(),
        remote_url: Some(remote_path.to_str().unwrap().to_string()),
        has_uncommitted: true, // Mark as dirty
        commit_timestamp: 0,
    };
    let mut by_remote = HashMap::new();
    by_remote.insert(remote_path.to_str().unwrap().to_string(), vec![instance]);
    let report = ValidationReport {
        total_nested: 1,
        by_remote,
        no_remote: vec![],
    };

    // 5. Try sync without stash/force (should fail/skip)
    let result = sync_subrepo_with_report("upstream", &commit2, false, false, &report);
    assert!(result.is_ok()); // sync_subrepo returns Ok even if it skips, but shows warning
                             // Verify it DID NOT sync
    assert_eq!(get_head_commit(&sub_path)?, commit1);

    // 6. Try sync WITH stash
    sync_subrepo_with_report("upstream", &commit2, true, false, &report)?;

    // Verify it DID sync
    assert_eq!(get_head_commit(&sub_path)?, commit2);

    // Verify stash was created (dirty.txt should be gone from worktree if stashed,
    // but the current implementation doesn't pop it back yet in sync_subrepo)
    // Actually, sync_subrepo just runs `git stash push`. It doesn't pop.
    assert!(!sub_path.join("dirty.txt").exists());

    Ok(())
}

#[test]
fn test_update_skips_diverged_local_commits() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();

    // 1. Upstream
    let remote_path = root.join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "f.txt", "v1", "Initial")?;
    let _base = get_head_commit(&remote_path)?;
    create_test_commit(&remote_path, "f.txt", "v2", "Remote Update")?;
    let remote_tip = get_head_commit(&remote_path)?;

    // 2. Parent with subrepo
    let parent_path = root.join("parent");
    std::fs::create_dir(&parent_path)?;
    setup_git_repo(&parent_path)?;
    let sub_path = parent_path.join("sub");
    clone_repo(&remote_path, &sub_path)?;
    // Create a local commit in subrepo to diverge
    create_test_commit(&sub_path, "local.txt", "data", "Local Commit")?;
    let local_tip = get_head_commit(&sub_path)?;

    // 3. Report
    let instance = SubrepoInstance {
        parent_repo: "parent".to_string(),
        parent_path: parent_path.clone(),
        subrepo_name: "upstream".to_string(),
        subrepo_path: sub_path.clone(),
        relative_path: "sub".to_string(),
        commit_hash: local_tip.clone(),
        short_hash: local_tip[..7].to_string(),
        remote_url: Some(remote_path.to_str().unwrap().to_string()),
        has_uncommitted: false,
        commit_timestamp: 0,
    };
    let mut by_remote = HashMap::new();
    by_remote.insert(remote_path.to_str().unwrap().to_string(), vec![instance]);
    let report = ValidationReport {
        total_nested: 1,
        by_remote,
        no_remote: vec![],
    };

    update_subrepo_with_report("upstream", false, &report)?;

    assert_eq!(
        get_head_commit(&sub_path)?,
        local_tip,
        "A normal update must not move away from divergent local commits"
    );
    assert_ne!(local_tip, remote_tip);

    Ok(())
}

#[test]
fn test_update_allows_fast_forward_commit() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();

    let remote_path = root.join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "f.txt", "v1", "Initial")?;
    let initial = get_head_commit(&remote_path)?;

    let parent_path = root.join("parent");
    std::fs::create_dir(&parent_path)?;
    setup_git_repo(&parent_path)?;
    let sub_path = parent_path.join("sub");
    clone_repo(&remote_path, &sub_path)?;

    create_test_commit(&remote_path, "f.txt", "v2", "Remote update")?;
    let remote_tip = get_head_commit(&remote_path)?;

    let instance = SubrepoInstance {
        parent_repo: "parent".to_string(),
        parent_path,
        subrepo_name: "upstream".to_string(),
        subrepo_path: sub_path.clone(),
        relative_path: "sub".to_string(),
        commit_hash: initial.clone(),
        short_hash: initial[..7].to_string(),
        remote_url: Some(remote_path.to_string_lossy().into_owned()),
        has_uncommitted: false,
        commit_timestamp: 0,
    };
    let report = ValidationReport {
        total_nested: 1,
        by_remote: HashMap::from([(remote_path.to_string_lossy().into_owned(), vec![instance])]),
        no_remote: Vec::new(),
    };

    update_subrepo_with_report("upstream", false, &report)?;
    assert_eq!(get_head_commit(&sub_path)?, remote_tip);
    Ok(())
}

#[test]
fn test_sync_with_conflicts_fails() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();

    let remote_path = root.join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "common.txt", "v1", "Init")?;
    let commit1 = get_head_commit(&remote_path)?;
    create_test_commit(&remote_path, "common.txt", "v2", "Remote Update")?;
    let commit2 = get_head_commit(&remote_path)?;

    let parent_path = root.join("parent");
    std::fs::create_dir(&parent_path)?;
    setup_git_repo(&parent_path)?;
    let sub_path = parent_path.join("sub");
    clone_repo(&remote_path, &sub_path)?;
    Command::new("git")
        .args(["-C", sub_path.to_str().unwrap(), "checkout", &commit1])
        .output()?;

    // Create a local file that will conflict with commit2
    // Actually, common.txt already exists. If I modify it locally and don't stash, checkout should fail.
    std::fs::write(sub_path.join("common.txt"), "local mod")?;

    let instance = SubrepoInstance {
        parent_repo: "parent".to_string(),
        parent_path: parent_path.clone(),
        subrepo_name: "upstream".to_string(),
        subrepo_path: sub_path.clone(),
        relative_path: "sub".to_string(),
        commit_hash: commit1.clone(),
        short_hash: commit1[..7].to_string(),
        remote_url: Some(remote_path.to_str().unwrap().to_string()),
        has_uncommitted: true,
        commit_timestamp: 0,
    };
    let mut by_remote = HashMap::new();
    by_remote.insert(remote_path.to_str().unwrap().to_string(), vec![instance]);
    let report = ValidationReport {
        total_nested: 1,
        by_remote,
        no_remote: vec![],
    };

    // Try sync with force=true (discards changes) - wait, does checkout -f discard changes?
    // The current checkout_commit doesn't use -f.
    // So even with force=true in sync_subrepo, it might still fail if checkout fails.
    // Actually, sync_subrepo with force=true just skips the has_uncommitted_changes check.

    let result = sync_subrepo_with_report("upstream", &commit2, false, true, &report);
    // It should return an error because checkout fails
    assert!(result.is_err());

    Ok(())
}

#[test]
fn test_sync_missing_remote_handled() -> Result<()> {
    let report = ValidationReport {
        total_nested: 0,
        by_remote: HashMap::new(),
        no_remote: vec![],
    };

    // Should bail with "not found"
    let result = sync_subrepo_with_report("nonexistent", "abc", false, false, &report);
    assert!(result.is_err());
    Ok(())
}

#[test]
fn test_multiple_subrepos_batch_sync() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();

    // Upstream
    let remote_path = root.join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "f.txt", "v1", "Init")?;
    let commit1 = get_head_commit(&remote_path)?;
    create_test_commit(&remote_path, "f.txt", "v2", "Update")?;
    let commit2 = get_head_commit(&remote_path)?;

    let mut by_remote = HashMap::new();
    let mut instances = Vec::new();

    // Create 3 parent repos
    for i in 1..=3 {
        let parent_name = format!("parent-{}", i);
        let parent_path = root.join(&parent_name);
        std::fs::create_dir(&parent_path)?;
        setup_git_repo(&parent_path)?;
        let sub_path = parent_path.join("sub");
        clone_repo(&remote_path, &sub_path)?;
        Command::new("git")
            .args(["-C", sub_path.to_str().unwrap(), "checkout", &commit1])
            .output()?;

        instances.push(SubrepoInstance {
            parent_repo: parent_name,
            parent_path: parent_path.clone(),
            subrepo_name: "upstream".to_string(),
            subrepo_path: sub_path,
            relative_path: "sub".to_string(),
            commit_hash: commit1.clone(),
            short_hash: commit1[..7].to_string(),
            remote_url: Some(remote_path.to_str().unwrap().to_string()),
            has_uncommitted: false,
            commit_timestamp: 0,
        });
    }

    by_remote.insert(remote_path.to_str().unwrap().to_string(), instances);
    let report = ValidationReport {
        total_nested: 3,
        by_remote,
        no_remote: vec![],
    };

    sync_subrepo_with_report("upstream", &commit2, false, false, &report)?;

    // Verify all 3 synced
    for i in 1..=3 {
        let sub_path = root.join(format!("parent-{}", i)).join("sub");
        assert_eq!(get_head_commit(&sub_path)?, commit2);
    }

    Ok(())
}
