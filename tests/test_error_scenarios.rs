use anyhow::Result;
use goobits_repos::git::{fetch_and_analyze_for_pull, pull_if_needed, Status};
use std::process::Command;
use tempfile::TempDir;

mod common;
use common::git::{clone_repo, create_test_commit, run_git_ok, setup_git_repo};

#[tokio::test]
async fn test_pull_merge_conflict_handled() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();

    // 1. Upstream
    let remote_path = root.join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "f.txt", "base", "Init")?;

    // 2. Clone to local
    let local_path = root.join("local");
    Command::new("git")
        .args([
            "clone",
            remote_path.to_str().unwrap(),
            local_path.to_str().unwrap(),
        ])
        .output()?;
    setup_git_repo(&local_path)?; // Re-setup to ensure test config (user.name etc)

    // 3. Update remote with a change
    create_test_commit(&remote_path, "f.txt", "remote change", "Remote Update")?;

    // 4. Update local with a CONFLICTING change
    create_test_commit(&local_path, "f.txt", "local conflict", "Local Update")?;

    // 5. Analyze for pull
    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;

    // It should detect diverged
    assert_eq!(fetch_result.status, Status::PullError);
    assert!(fetch_result.message.contains("diverged"));

    // 6. Try pull (should return error immediately because status is PullError)
    let (status, message, _) = pull_if_needed(&local_path, &fetch_result, false).await;
    assert_eq!(status, Status::PullError);
    assert!(message.contains("diverged"));

    Ok(())
}

#[tokio::test]
async fn test_rebase_conflict_restores_original_checkout() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();
    let remote_path = root.join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "shared.txt", "base", "Init")?;

    let local_path = root.join("local");
    Command::new("git")
        .args([
            "clone",
            remote_path
                .to_str()
                .expect("temporary path should be UTF-8"),
            local_path.to_str().expect("temporary path should be UTF-8"),
        ])
        .output()?;
    setup_git_repo(&local_path)?;

    create_test_commit(&remote_path, "shared.txt", "remote change", "Remote update")?;
    create_test_commit(&local_path, "shared.txt", "local change", "Local update")?;
    let original_head = git_output(&local_path, &["rev-parse", "HEAD"])?;
    let original_branch = git_output(&local_path, &["symbolic-ref", "--short", "HEAD"])?;

    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(fetch_result.status, Status::PullError);

    let (status, message, _) = pull_if_needed(&local_path, &fetch_result, true).await;
    assert_eq!(status, Status::PullError);
    assert!(
        message.contains("aborted and restored original checkout"),
        "{message}"
    );
    assert_eq!(
        git_output(&local_path, &["rev-parse", "HEAD"])?,
        original_head
    );
    assert_eq!(
        git_output(&local_path, &["symbolic-ref", "--short", "HEAD"])?,
        original_branch
    );
    assert!(git_output(&local_path, &["status", "--porcelain"])?.is_empty());

    let rebase_head = Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", "REBASE_HEAD"])
        .current_dir(&local_path)
        .output()?;
    assert!(!rebase_head.status.success());

    Ok(())
}

#[tokio::test]
async fn test_pull_integrates_the_analyzed_upstream_snapshot() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();
    let remote_path = root.join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "base.txt", "base", "Init")?;

    let local_path = root.join("local");
    Command::new("git")
        .args([
            "clone",
            remote_path
                .to_str()
                .expect("temporary path should be UTF-8"),
            local_path.to_str().expect("temporary path should be UTF-8"),
        ])
        .output()?;
    setup_git_repo(&local_path)?;

    create_test_commit(&remote_path, "first.txt", "first", "First remote update")?;
    let analyzed_commit = git_output(&remote_path, &["rev-parse", "HEAD"])?;
    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(fetch_result.status, Status::Synced);
    assert_eq!(fetch_result.behind_count, 1);

    create_test_commit(&remote_path, "second.txt", "second", "Second remote update")?;
    let newer_remote_commit = git_output(&remote_path, &["rev-parse", "HEAD"])?;
    assert_ne!(newer_remote_commit, analyzed_commit);
    run_git_ok(&local_path, &["fetch", "origin"]);
    assert_eq!(
        git_output(&local_path, &["rev-parse", "@{upstream}"])?,
        newer_remote_commit
    );

    let (status, _, _) = pull_if_needed(&local_path, &fetch_result, false).await;
    assert_eq!(status, Status::Pulled);
    assert_eq!(
        git_output(&local_path, &["rev-parse", "HEAD"])?,
        analyzed_commit
    );
    assert!(local_path.join("first.txt").is_file());
    assert!(!local_path.join("second.txt").exists());

    Ok(())
}

#[tokio::test]
async fn test_pull_rejects_worktree_changed_after_analysis() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let remote_path = temp_dir.path().join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "base.txt", "base", "Init")?;

    let local_path = temp_dir.path().join("local");
    clone_repo(&remote_path, &local_path)?;
    create_test_commit(&remote_path, "remote.txt", "remote", "Remote update")?;

    let original_head = git_output(&local_path, &["rev-parse", "HEAD"])?;
    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(fetch_result.behind_count, 1);
    std::fs::write(local_path.join("untracked.txt"), "new work")?;

    let (status, message, _) = pull_if_needed(&local_path, &fetch_result, false).await;
    assert_eq!(status, Status::PullError);
    assert!(
        message.contains("worktree changed after pull analysis"),
        "{message}"
    );
    assert_eq!(
        git_output(&local_path, &["rev-parse", "HEAD"])?,
        original_head
    );
    assert!(!local_path.join("remote.txt").exists());

    Ok(())
}

#[tokio::test]
async fn test_pull_without_upstream_reports_no_tracking() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let remote_path = temp_dir.path().join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "remote.txt", "remote", "Remote init")?;

    let local_path = temp_dir.path().join("local");
    std::fs::create_dir(&local_path)?;
    setup_git_repo(&local_path)?;
    create_test_commit(&local_path, "local.txt", "local", "Local init")?;
    run_git_ok(
        &local_path,
        &[
            "remote",
            "add",
            "origin",
            remote_path
                .to_str()
                .expect("temporary path should be UTF-8"),
        ],
    );
    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(fetch_result.status, Status::NoUpstream);
    assert_eq!(fetch_result.message, "no tracking");

    Ok(())
}

#[tokio::test]
async fn test_pull_supports_a_local_dot_upstream() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    setup_git_repo(repo_path)?;
    create_test_commit(repo_path, "base.txt", "base", "Init")?;
    let main_branch = git_output(repo_path, &["branch", "--show-current"])?;
    run_git_ok(repo_path, &["switch", "-c", "upstream"]);
    create_test_commit(repo_path, "upstream.txt", "upstream", "Upstream update")?;
    run_git_ok(repo_path, &["switch", &main_branch]);
    run_git_ok(
        repo_path,
        &["branch", "--set-upstream-to=upstream", &main_branch],
    );
    run_git_ok(
        repo_path,
        &[
            "remote",
            "add",
            "origin",
            repo_path.to_str().expect("temporary path should be UTF-8"),
        ],
    );

    let fetch_result = fetch_and_analyze_for_pull(repo_path).await;
    assert_eq!(fetch_result.status, Status::Synced);
    assert_eq!(fetch_result.behind_count, 1);
    let (status, message, _) = pull_if_needed(repo_path, &fetch_result, false).await;
    assert_eq!(status, Status::Pulled, "{message}");
    assert!(repo_path.join("upstream.txt").is_file());

    Ok(())
}

#[tokio::test]
async fn test_pull_restores_a_missing_remote_tracking_ref() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let remote_path = temp_dir.path().join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "base.txt", "base", "Init")?;

    let local_path = temp_dir.path().join("local");
    clone_repo(&remote_path, &local_path)?;
    create_test_commit(&remote_path, "remote.txt", "remote", "Remote update")?;
    let upstream_name = git_output(&local_path, &["rev-parse", "--abbrev-ref", "@{upstream}"])?;
    run_git_ok(
        &local_path,
        &["update-ref", "-d", &format!("refs/remotes/{upstream_name}")],
    );

    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(fetch_result.status, Status::Synced);
    assert_eq!(fetch_result.behind_count, 1);
    let (status, _, _) = pull_if_needed(&local_path, &fetch_result, false).await;
    assert_eq!(status, Status::Pulled);
    assert_eq!(
        git_output(&local_path, &["rev-parse", "HEAD"])?,
        git_output(&remote_path, &["rev-parse", "HEAD"])?
    );

    Ok(())
}

#[tokio::test]
async fn test_rebase_resolves_a_missing_upstream_by_its_full_ref() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let remote_path = temp_dir.path().join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "base.txt", "base", "Init")?;

    let local_path = temp_dir.path().join("local");
    clone_repo(&remote_path, &local_path)?;
    create_test_commit(&local_path, "local.txt", "preserve me", "Local work")?;
    let upstream_name = git_output(&local_path, &["rev-parse", "--abbrev-ref", "@{upstream}"])?;
    run_git_ok(&local_path, &["branch", &upstream_name, "HEAD"]);
    run_git_ok(
        &local_path,
        &["branch", &format!("remotes/{upstream_name}"), "HEAD"],
    );
    run_git_ok(
        &local_path,
        &["update-ref", "-d", &format!("refs/remotes/{upstream_name}")],
    );
    create_test_commit(&remote_path, "remote.txt", "remote", "Remote update")?;

    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(fetch_result.status, Status::PullError);
    assert_eq!(fetch_result.behind_count, 1);
    let (status, message, _) = pull_if_needed(&local_path, &fetch_result, true).await;
    assert_eq!(status, Status::Pulled, "{message}");
    assert_eq!(
        std::fs::read_to_string(local_path.join("local.txt"))?,
        "preserve me"
    );
    assert_eq!(
        git_output(&local_path, &["show", "-s", "--format=%s", "HEAD"])?,
        "Local work"
    );

    Ok(())
}

#[tokio::test]
async fn test_rebase_uses_the_pre_fetch_upstream_as_the_fork_point() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let remote_path = temp_dir.path().join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "base.txt", "base", "Base")?;
    let base_commit = git_output(&remote_path, &["rev-parse", "HEAD"])?;
    create_test_commit(
        &remote_path,
        "old-upstream.txt",
        "old",
        "Dropped upstream commit",
    )?;
    let dropped_upstream = git_output(&remote_path, &["rev-parse", "HEAD"])?;

    let local_path = temp_dir.path().join("local");
    clone_repo(&remote_path, &local_path)?;
    create_test_commit(&local_path, "local.txt", "local", "Local work")?;

    run_git_ok(&remote_path, &["reset", "--hard", &base_commit]);
    create_test_commit(
        &remote_path,
        "rewritten-upstream.txt",
        "replacement",
        "Rewritten upstream",
    )?;
    let rewritten_upstream = git_output(&remote_path, &["rev-parse", "HEAD"])?;

    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(fetch_result.status, Status::PullError);
    assert!(fetch_result.ahead_count > 0);
    assert_eq!(fetch_result.behind_count, 1);

    let (status, _, _) = pull_if_needed(&local_path, &fetch_result, true).await;
    assert_eq!(status, Status::Pulled);
    assert_eq!(
        git_output(&local_path, &["rev-parse", "HEAD^"])?,
        rewritten_upstream
    );
    assert_eq!(
        git_output(&local_path, &["show", "-s", "--format=%s", "HEAD"])?,
        "Local work"
    );
    let dropped_is_ancestor = Command::new("git")
        .args(["merge-base", "--is-ancestor", &dropped_upstream, "HEAD"])
        .current_dir(&local_path)
        .status()?;
    assert!(!dropped_is_ancestor.success());

    Ok(())
}

#[tokio::test]
async fn test_pull_fetches_lfs_objects_for_the_pinned_target() -> Result<()> {
    if !git_lfs_available() {
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    let remote_path = temp_dir.path().join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "base.txt", "base", "Init")?;

    let local_path = temp_dir.path().join("local");
    clone_repo(&remote_path, &local_path)?;
    run_git_ok(&local_path, &["lfs", "install", "--local"]);
    run_git_ok(&local_path, &["config", "lfs.fetchexclude", "asset.bin"]);
    let (target_commit, object_id) = commit_lfs_file(&remote_path, "large object")?;
    assert!(!lfs_object_path(&local_path, &object_id).exists());

    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(fetch_result.behind_count, 1);
    let (status, message, _) = pull_if_needed(&local_path, &fetch_result, false).await;
    assert_eq!(status, Status::Pulled, "{message}");
    assert!(message.contains("with LFS"));
    assert_eq!(
        git_output(&local_path, &["rev-parse", "HEAD"])?,
        target_commit
    );
    assert!(local_path.join("asset.bin").is_file());
    assert!(lfs_object_path(&local_path, &object_id).exists());

    Ok(())
}

#[tokio::test]
async fn test_failed_target_lfs_fetch_leaves_head_unchanged() -> Result<()> {
    if !git_lfs_available() {
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    let remote_path = temp_dir.path().join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "base.txt", "base", "Init")?;

    let local_path = temp_dir.path().join("local");
    clone_repo(&remote_path, &local_path)?;
    run_git_ok(&local_path, &["lfs", "install", "--local"]);
    let (_, object_id) = commit_lfs_file(&remote_path, "missing object")?;
    std::fs::remove_file(lfs_object_path(&remote_path, &object_id))?;

    let original_head = git_output(&local_path, &["rev-parse", "HEAD"])?;
    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(fetch_result.behind_count, 1);
    let (status, message, _) = pull_if_needed(&local_path, &fetch_result, false).await;
    assert_eq!(status, Status::PullError);
    assert!(message.to_ascii_lowercase().contains("lfs"), "{message}");
    assert_eq!(
        git_output(&local_path, &["rev-parse", "HEAD"])?,
        original_head
    );
    assert!(!local_path.join("asset.bin").exists());

    Ok(())
}

#[tokio::test]
async fn test_changed_lfs_remote_url_is_rejected_before_checkout() -> Result<()> {
    if !git_lfs_available() {
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    let remote_path = temp_dir.path().join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "base.txt", "base", "Init")?;

    let local_path = temp_dir.path().join("local");
    clone_repo(&remote_path, &local_path)?;
    run_git_ok(&local_path, &["lfs", "install", "--local"]);
    commit_lfs_file(&remote_path, "large object")?;

    let original_head = git_output(&local_path, &["rev-parse", "HEAD"])?;
    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(fetch_result.behind_count, 1);

    let other_remote = temp_dir.path().join("other-upstream");
    std::fs::create_dir(&other_remote)?;
    setup_git_repo(&other_remote)?;
    create_test_commit(&other_remote, "other.txt", "other", "Other init")?;
    run_git_ok(
        &local_path,
        &[
            "remote",
            "set-url",
            "origin",
            other_remote
                .to_str()
                .expect("temporary path should be UTF-8"),
        ],
    );

    let (status, message, _) = pull_if_needed(&local_path, &fetch_result, false).await;
    assert_eq!(status, Status::PullError);
    assert!(message.contains("fetch remote URL changed"), "{message}");
    assert_eq!(
        git_output(&local_path, &["rev-parse", "HEAD"])?,
        original_head
    );
    assert!(!local_path.join("asset.bin").exists());

    Ok(())
}

fn git_lfs_available() -> bool {
    Command::new("git")
        .args(["lfs", "version"])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn commit_lfs_file(path: &std::path::Path, contents: &str) -> Result<(String, String)> {
    run_git_ok(path, &["lfs", "install", "--local"]);
    std::fs::write(
        path.join(".gitattributes"),
        "*.bin filter=lfs diff=lfs merge=lfs -text\n",
    )?;
    std::fs::write(path.join("asset.bin"), contents)?;
    run_git_ok(path, &["add", ".gitattributes", "asset.bin"]);
    run_git_ok(path, &["commit", "-m", "Add LFS object"]);

    let pointer = git_output(path, &["show", "HEAD:asset.bin"])?;
    let object_id = pointer
        .lines()
        .find_map(|line| line.strip_prefix("oid sha256:"))
        .ok_or_else(|| anyhow::anyhow!("LFS pointer omitted its object ID"))?
        .to_string();
    Ok((git_output(path, &["rev-parse", "HEAD"])?, object_id))
}

fn lfs_object_path(path: &std::path::Path, object_id: &str) -> std::path::PathBuf {
    path.join(".git")
        .join("lfs")
        .join("objects")
        .join(&object_id[..2])
        .join(&object_id[2..4])
        .join(object_id)
}

#[tokio::test]
async fn test_pull_refuses_a_branch_change_after_analysis() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();
    let remote_path = root.join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "base.txt", "base", "Init")?;

    let local_path = root.join("local");
    Command::new("git")
        .args([
            "clone",
            remote_path
                .to_str()
                .expect("temporary path should be UTF-8"),
            local_path.to_str().expect("temporary path should be UTF-8"),
        ])
        .output()?;
    setup_git_repo(&local_path)?;
    create_test_commit(&remote_path, "remote.txt", "remote", "Remote update")?;

    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(fetch_result.behind_count, 1);
    git_output(&local_path, &["checkout", "-b", "other"])?;
    let switched_head = git_output(&local_path, &["rev-parse", "HEAD"])?;

    let (status, message, _) = pull_if_needed(&local_path, &fetch_result, false).await;
    assert_eq!(status, Status::PullError);
    assert!(
        message.contains("branch changed after pull analysis"),
        "{message}"
    );
    assert_eq!(
        git_output(&local_path, &["rev-parse", "HEAD"])?,
        switched_head
    );
    assert!(!local_path.join("remote.txt").exists());

    Ok(())
}

#[tokio::test]
async fn test_rebase_refuses_a_head_change_after_analysis() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();
    let remote_path = root.join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "base.txt", "base", "Init")?;

    let local_path = root.join("local");
    Command::new("git")
        .args([
            "clone",
            remote_path
                .to_str()
                .expect("temporary path should be UTF-8"),
            local_path.to_str().expect("temporary path should be UTF-8"),
        ])
        .output()?;
    setup_git_repo(&local_path)?;
    create_test_commit(&remote_path, "remote.txt", "remote", "Remote update")?;

    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(fetch_result.behind_count, 1);
    create_test_commit(&local_path, "local.txt", "local", "Late local update")?;
    let changed_head = git_output(&local_path, &["rev-parse", "HEAD"])?;

    let (status, message, _) = pull_if_needed(&local_path, &fetch_result, true).await;
    assert_eq!(status, Status::PullError);
    assert!(
        message.contains("HEAD changed after pull analysis"),
        "{message}"
    );
    assert_eq!(
        git_output(&local_path, &["rev-parse", "HEAD"])?,
        changed_head
    );
    assert!(!local_path.join("remote.txt").exists());

    Ok(())
}

#[tokio::test]
async fn test_pull_treats_an_option_like_remote_name_literally() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let remote_path = temp_dir.path().join("upstream");
    std::fs::create_dir(&remote_path)?;
    setup_git_repo(&remote_path)?;
    create_test_commit(&remote_path, "base.txt", "base", "Init")?;

    let local_path = temp_dir.path().join("local");
    clone_repo(&remote_path, &local_path)?;
    run_git_ok(&local_path, &["remote", "remove", "origin"]);
    run_git_ok(
        &local_path,
        &[
            "remote",
            "add",
            "--",
            "--all",
            remote_path
                .to_str()
                .expect("temporary path should be UTF-8"),
        ],
    );
    let missing_remote = temp_dir.path().join("missing.git");
    run_git_ok(
        &local_path,
        &[
            "remote",
            "add",
            "decoy",
            missing_remote
                .to_str()
                .expect("temporary path should be UTF-8"),
        ],
    );
    let branch = git_output(&local_path, &["branch", "--show-current"])?;
    run_git_ok(
        &local_path,
        &["config", &format!("branch.{branch}.remote"), "--all"],
    );
    run_git_ok(
        &local_path,
        &[
            "config",
            &format!("branch.{branch}.merge"),
            &format!("refs/heads/{branch}"),
        ],
    );
    create_test_commit(&remote_path, "remote.txt", "remote", "Remote update")?;

    let fetch_result = fetch_and_analyze_for_pull(&local_path).await;
    assert_eq!(
        fetch_result.status,
        Status::Synced,
        "{}",
        fetch_result.message
    );
    assert_eq!(fetch_result.behind_count, 1);
    let (status, message, _) = pull_if_needed(&local_path, &fetch_result, false).await;
    assert_eq!(status, Status::Pulled, "{message}");
    assert!(local_path.join("remote.txt").exists());

    Ok(())
}

fn git_output(path: &std::path::Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).current_dir(path).output()?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[tokio::test]
async fn test_invalid_git_repo_detection() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();

    // Not a git repo
    std::fs::write(repo_path.join("README.md"), "hello")?;

    let fetch_result = fetch_and_analyze_for_pull(repo_path).await;
    assert_eq!(fetch_result.status, Status::Error);

    Ok(())
}

#[tokio::test]
async fn test_corrupt_git_repo_handled() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    setup_git_repo(repo_path)?;

    // Corrupt it by removing .git/objects
    let objects_dir = repo_path.join(".git").join("objects");
    if objects_dir.exists() {
        std::fs::remove_dir_all(objects_dir)?;
    }

    let fetch_result = fetch_and_analyze_for_pull(repo_path).await;
    assert_eq!(fetch_result.status, Status::Error);

    Ok(())
}
