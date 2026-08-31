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

#[tokio::test]
async fn test_sync_with_uncommitted_changes_stash() -> Result<()> {
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
        checkout_kind: goobits_repos::subrepo::NestedCheckoutKind::Independent,
    };
    let mut by_remote = HashMap::new();
    by_remote.insert(remote_path.to_str().unwrap().to_string(), vec![instance]);
    let report = ValidationReport {
        total_nested: 1,
        by_remote,
        no_remote: vec![],
        uninitialized_submodules: vec![],
    };

    // 5. Try sync without stash/force (should fail/skip)
    let result = sync_subrepo_with_report("upstream", &commit2, false, false, &report).await;
    assert!(result.is_ok()); // sync_subrepo returns Ok even if it skips, but shows warning
                             // Verify it DID NOT sync
    assert_eq!(get_head_commit(&sub_path)?, commit1);

    // 6. Try sync WITH stash
    sync_subrepo_with_report("upstream", &commit2, true, false, &report).await?;

    // Verify it DID sync
    assert_eq!(get_head_commit(&sub_path)?, commit2);

    // Verify stash was created (dirty.txt should be gone from worktree if stashed,
    // but the current implementation doesn't pop it back yet in sync_subrepo)
    // Actually, sync_subrepo just runs `git stash push`. It doesn't pop.
    assert!(!sub_path.join("dirty.txt").exists());

    Ok(())
}

#[tokio::test]
async fn test_update_skips_diverged_local_commits() -> Result<()> {
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
        checkout_kind: goobits_repos::subrepo::NestedCheckoutKind::Independent,
    };
    let mut by_remote = HashMap::new();
    by_remote.insert(remote_path.to_str().unwrap().to_string(), vec![instance]);
    let report = ValidationReport {
        total_nested: 1,
        by_remote,
        no_remote: vec![],
        uninitialized_submodules: vec![],
    };

    update_subrepo_with_report("upstream", false, &report).await?;

    assert_eq!(
        get_head_commit(&sub_path)?,
        local_tip,
        "A normal update must not move away from divergent local commits"
    );
    assert_ne!(local_tip, remote_tip);

    Ok(())
}

#[tokio::test]
async fn test_update_allows_fast_forward_commit() -> Result<()> {
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
        checkout_kind: goobits_repos::subrepo::NestedCheckoutKind::Independent,
    };
    let report = ValidationReport {
        total_nested: 1,
        by_remote: HashMap::from([(remote_path.to_string_lossy().into_owned(), vec![instance])]),
        no_remote: Vec::new(),
        uninitialized_submodules: Vec::new(),
    };

    update_subrepo_with_report("upstream", false, &report).await?;
    assert_eq!(get_head_commit(&sub_path)?, remote_tip);
    Ok(())
}

#[tokio::test]
async fn test_update_uses_advertised_remote_head_after_default_branch_change() -> Result<()> {
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

    let checkout = Command::new("git")
        .args(["checkout", "-b", "release"])
        .current_dir(&remote_path)
        .output()?;
    assert!(checkout.status.success());
    create_test_commit(&remote_path, "f.txt", "v2", "New default branch")?;
    let release_tip = get_head_commit(&remote_path)?;

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
        checkout_kind: goobits_repos::subrepo::NestedCheckoutKind::Independent,
    };
    let report = ValidationReport {
        total_nested: 1,
        by_remote: HashMap::from([(remote_path.to_string_lossy().into_owned(), vec![instance])]),
        no_remote: Vec::new(),
        uninitialized_submodules: Vec::new(),
    };

    update_subrepo_with_report("upstream", false, &report).await?;

    assert_eq!(get_head_commit(&sub_path)?, release_tip);
    assert_ne!(release_tip, initial);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn test_update_accepts_non_utf8_checkout_paths() -> Result<()> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

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
    let sub_path = parent_path.join(OsString::from_vec(b"sub-\xff".to_vec()));
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
        checkout_kind: goobits_repos::subrepo::NestedCheckoutKind::Independent,
    };
    let report = ValidationReport {
        total_nested: 1,
        by_remote: HashMap::from([(remote_path.to_string_lossy().into_owned(), vec![instance])]),
        no_remote: Vec::new(),
        uninitialized_submodules: Vec::new(),
    };

    update_subrepo_with_report("upstream", false, &report).await?;

    assert_eq!(get_head_commit(&sub_path)?, remote_tip);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn test_update_uses_one_immutable_target_for_every_copy() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();
    let remote_path = root.join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "f.txt", "v1", "Initial")?;
    let initial = get_head_commit(&remote_path)?;

    let mut instances = Vec::new();
    for name in ["parent-a", "parent-b"] {
        let parent_path = root.join(name);
        std::fs::create_dir(&parent_path)?;
        setup_git_repo(&parent_path)?;
        let subrepo_path = parent_path.join("sub");
        clone_repo(&remote_path, &subrepo_path)?;
        instances.push(SubrepoInstance {
            parent_repo: name.to_string(),
            parent_path,
            subrepo_name: "upstream".to_string(),
            subrepo_path,
            relative_path: "sub".to_string(),
            commit_hash: initial.clone(),
            short_hash: initial[..7].to_string(),
            remote_url: Some(remote_path.to_string_lossy().into_owned()),
            has_uncommitted: false,
            commit_timestamp: 0,
            checkout_kind: goobits_repos::subrepo::NestedCheckoutKind::Independent,
        });
    }

    create_test_commit(&remote_path, "f.txt", "v2", "Remote update")?;
    let selected_target = get_head_commit(&remote_path)?;
    let marker = root.join("remote-advanced-during-update");
    let hook = instances[0].subrepo_path.join(".git/hooks/post-checkout");
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\nif [ ! -f '{}' ]; then\n  touch '{}'\n  printf 'v3' > '{}/f.txt'\n  git -C '{}' add f.txt\n  git -C '{}' commit -m 'Racing update' >/dev/null\nfi\n",
            marker.display(),
            marker.display(),
            remote_path.display(),
            remote_path.display(),
            remote_path.display(),
        ),
    )?;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))?;

    let report = ValidationReport {
        total_nested: 2,
        by_remote: HashMap::from([(
            remote_path.to_string_lossy().into_owned(),
            instances.clone(),
        )]),
        no_remote: Vec::new(),
        uninitialized_submodules: Vec::new(),
    };

    update_subrepo_with_report("upstream", false, &report).await?;

    assert!(marker.exists(), "race hook did not advance the remote");
    assert_ne!(get_head_commit(&remote_path)?, selected_target);
    for instance in instances {
        assert_eq!(
            get_head_commit(&instance.subrepo_path)?,
            selected_target,
            "every copy must use the target selected before mutation"
        );
    }
    Ok(())
}

#[tokio::test]
async fn test_sync_force_discards_tracked_changes() -> Result<()> {
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
        checkout_kind: goobits_repos::subrepo::NestedCheckoutKind::Independent,
    };
    let mut by_remote = HashMap::new();
    by_remote.insert(remote_path.to_str().unwrap().to_string(), vec![instance]);
    let report = ValidationReport {
        total_nested: 1,
        by_remote,
        no_remote: vec![],
        uninitialized_submodules: vec![],
    };

    sync_subrepo_with_report("upstream", &commit2, false, true, &report).await?;
    assert_eq!(get_head_commit(&sub_path)?, commit2);
    assert_eq!(std::fs::read_to_string(sub_path.join("common.txt"))?, "v2");

    Ok(())
}

#[tokio::test]
async fn test_sync_missing_remote_handled() -> Result<()> {
    let report = ValidationReport {
        total_nested: 0,
        by_remote: HashMap::new(),
        no_remote: vec![],
        uninitialized_submodules: vec![],
    };

    // Should bail with "not found"
    let result = sync_subrepo_with_report("nonexistent", "abc", false, false, &report).await;
    assert!(result.is_err());
    Ok(())
}

#[tokio::test]
async fn test_multiple_subrepos_batch_sync() -> Result<()> {
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
            checkout_kind: goobits_repos::subrepo::NestedCheckoutKind::Independent,
        });
    }

    by_remote.insert(remote_path.to_str().unwrap().to_string(), instances);
    let report = ValidationReport {
        total_nested: 3,
        by_remote,
        no_remote: vec![],
        uninitialized_submodules: vec![],
    };

    sync_subrepo_with_report("upstream", &commit2, false, false, &report).await?;

    // Verify all 3 synced
    for i in 1..=3 {
        let sub_path = root.join(format!("parent-{}", i)).join("sub");
        assert_eq!(get_head_commit(&sub_path)?, commit2);
    }

    Ok(())
}
