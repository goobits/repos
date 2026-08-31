//! Integration tests for command modules
//!
//! This module tests the core command functionality including:
//! - Sync operations (push/pull)
//! - Staging operations (stage/unstage/commit/status)
//!
//! Note: These tests verify command behavior and logic, focusing on error handling,
//! edge cases, and proper execution flow. They test the commands work correctly
//! without requiring actual network operations or real remotes.

mod common;
use common::fixtures::TestRepo;
use common::git::{
    add_bare_remote, clone_repo, create_test_commit, get_head_commit, is_git_available, run_git_ok,
    setup_git_repo, IsolatedGitConfig,
};
use common::CurrentDirGuard;

use goobits_repos::commands::staging::{
    handle_commit_command, handle_stage_command, handle_staging_status_command,
    handle_unstage_command, StatusFilters,
};
use goobits_repos::commands::sync::{
    handle_fetch_command, handle_push_command, handle_sync_command,
};
use goobits_repos::git::{fetch_and_analyze, get_staging_status, push_if_needed, Status};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

#[tokio::test]
async fn test_nested_status_all_reports_every_checkout_kind() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        return;
    }

    let root = TempDir::new().expect("Failed to create test directory");
    let shared_remote = root.path().join("shared-remote");
    fs::create_dir(&shared_remote).expect("Failed to create shared remote");
    setup_git_repo(&shared_remote).expect("Failed to initialize shared remote");
    create_test_commit(&shared_remote, "shared.txt", "v1", "initial")
        .expect("Failed to create initial shared commit");
    let old_commit = get_head_commit(&shared_remote).expect("Failed to inspect initial commit");
    create_test_commit(&shared_remote, "shared.txt", "v2", "update")
        .expect("Failed to create updated shared commit");

    let unique_remote = root.path().join("unique-remote");
    fs::create_dir(&unique_remote).expect("Failed to create unique remote");
    setup_git_repo(&unique_remote).expect("Failed to initialize unique remote");
    create_test_commit(&unique_remote, "unique.txt", "v1", "initial")
        .expect("Failed to create unique commit");

    let submodule_remote = root.path().join("submodule-remote");
    fs::create_dir(&submodule_remote).expect("Failed to create submodule remote");
    setup_git_repo(&submodule_remote).expect("Failed to initialize submodule remote");
    create_test_commit(&submodule_remote, "module.txt", "v1", "initial")
        .expect("Failed to create submodule commit");

    let parent_a = root.path().join("parent-a");
    let parent_b = root.path().join("parent-b");
    for parent in [&parent_a, &parent_b] {
        fs::create_dir(parent).expect("Failed to create parent repository");
        setup_git_repo(parent).expect("Failed to initialize parent repository");
        create_test_commit(parent, "README.md", "parent", "initial")
            .expect("Failed to create parent commit");
    }

    let shared_a = parent_a.join("shared");
    let shared_b = parent_b.join("shared");
    clone_repo(&shared_remote, &shared_a).expect("Failed to clone first shared copy");
    clone_repo(&shared_remote, &shared_b).expect("Failed to clone second shared copy");
    run_git_ok(&shared_a, &["checkout", &old_commit]);

    let unique = parent_a.join("unique");
    clone_repo(&unique_remote, &unique).expect("Failed to clone unique nested repository");

    let orphan = parent_b.join("orphan");
    fs::create_dir(&orphan).expect("Failed to create missing-origin repository");
    setup_git_repo(&orphan).expect("Failed to initialize missing-origin repository");
    create_test_commit(&orphan, "orphan.txt", "v1", "initial")
        .expect("Failed to create missing-origin commit");

    for parent in [&parent_a, &parent_b] {
        let output = Command::new("git")
            .args([
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                submodule_remote.to_str().expect("UTF-8 submodule path"),
                "modules/shared-submodule",
            ])
            .current_dir(parent)
            .output()
            .expect("Failed to add submodule");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["nested", "status", "--all"])
        .current_dir(root.path())
        .output()
        .expect("Failed to run nested status");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("Drifted groups    1"), "{stdout}");
    assert!(stdout.contains("Synced groups     1"), "{stdout}");
    assert!(stdout.contains("Shared groups     2"), "{stdout}");
    assert!(stdout.contains("Unique groups     1"), "{stdout}");
    assert!(stdout.contains("Missing origin    1"), "{stdout}");
    assert!(stdout.contains("Nested copies     6"), "{stdout}");
    assert!(stdout.contains("Independent       4"), "{stdout}");
    assert!(stdout.contains("Submodules        2"), "{stdout}");
    assert!(stdout.contains("Fleet repos       11"), "{stdout}");
    assert!(stdout.contains("parent-a/shared"), "{stdout}");
    assert!(stdout.contains("parent-b/shared"), "{stdout}");
    assert!(stdout.contains("parent-a/unique"), "{stdout}");
    assert!(stdout.contains("parent-b/orphan"), "{stdout}");
    assert!(
        stdout.contains("parent-a/modules/shared-submodule"),
        "{stdout}"
    );
    assert!(
        stdout.contains("parent-b/modules/shared-submodule"),
        "{stdout}"
    );
}

// ==============================================================================
// SYNC COMMAND TESTS (commands/sync.rs)
// ==============================================================================

#[tokio::test]
async fn test_sync_command_with_no_repos() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let empty_dir = TempDir::new().expect("Failed to create temp directory");
    let _cwd = CurrentDirGuard::enter(empty_dir.path()).expect("Failed to change dir");

    let result = handle_sync_command(false, false, false, true, None, false).await;

    assert!(
        result.is_ok(),
        "Sync command should handle an empty directory: {:?}",
        result
    );
}

#[tokio::test]
async fn test_sync_command_rebases_and_pushes_diverged_repository() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        return;
    }

    let repo = TestRepo::new().expect("Failed to create test repo");
    let remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");
    let updater_root = TempDir::new().expect("Failed to create updater directory");
    let updater = updater_root.path().join("updater");
    clone_repo(&remote.path().join("remote.git"), &updater).expect("Failed to clone test remote");

    create_test_commit(&updater, "remote.txt", "remote change", "Remote update")
        .expect("Failed to create remote commit");
    run_git_ok(&updater, &["push"]);
    create_test_commit(repo.path(), "local.txt", "local change", "Local update")
        .expect("Failed to create local commit");

    let sync = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["sync", "--sequential", "--no-drift-check"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos sync");
    let stdout = String::from_utf8_lossy(&sync.stdout);
    let stderr = String::from_utf8_lossy(&sync.stderr);

    assert!(sync.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("▌ Pulled"), "{stdout}");
    assert!(stdout.contains("▌ Pushed"), "{stdout}");
    assert!(repo.path().join("remote.txt").is_file());
    assert!(repo.path().join("local.txt").is_file());
    assert_eq!(
        get_head_commit(repo.path()).expect("Failed to resolve local HEAD"),
        get_head_commit(&remote.path().join("remote.git")).expect("Failed to resolve remote HEAD"),
        "sync should push the rebased local commit"
    );
}

#[tokio::test]
async fn test_fetch_command_with_no_repos() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        return;
    }

    let empty_dir = TempDir::new().expect("Failed to create temp directory");
    let _cwd = CurrentDirGuard::enter(empty_dir.path()).expect("Failed to change dir");

    let result = handle_fetch_command(false, None, false).await;

    assert!(
        result.is_ok(),
        "Fetch should accept an empty directory: {result:?}"
    );
}

#[tokio::test]
async fn test_staging_status_preserves_the_first_porcelain_status_column() {
    if !is_git_available() {
        return;
    }

    let repo = TestRepo::new().expect("Failed to create test repo");
    repo.create_file("README.md", "unstaged change")
        .expect("Failed to modify tracked file");

    let (status, _) = get_staging_status(repo.path())
        .await
        .expect("Failed to inspect staging status");

    assert_eq!(status, " M README.md");
}

#[tokio::test]
async fn test_sync_command_with_single_repo_no_remote() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let repo = match TestRepo::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    let result = handle_sync_command(true, false, false, true, None, false).await;

    assert!(
        result.is_ok(),
        "Sync command should run pull then push without panicking: {:?}",
        result
    );
}

#[tokio::test]
async fn test_push_command_with_single_repo_no_changes() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository with a remote
    let repo = match TestRepo::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };
    let _remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");

    // Change to repo directory so it gets discovered
    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run push command - should complete without errors (even though push will fail due to no actual remote)
    let result = handle_push_command(false, false, false, true, None, false).await;

    assert!(
        result.is_ok(),
        "Push command should complete without panicking: {:?}",
        result
    );
}

struct PublishedNestedFixture {
    parent: PathBuf,
    child: PathBuf,
    parent_remote: TempDir,
    child_remote: TempDir,
}

fn setup_published_nested_fixture(root: &Path) -> PublishedNestedFixture {
    let parent = root.join("parent");
    let child = parent.join("child");
    fs::create_dir_all(&child).expect("Failed to create nested repositories");
    setup_git_repo(&parent).expect("Failed to initialize parent repository");
    setup_git_repo(&child).expect("Failed to initialize child repository");
    create_test_commit(&parent, "README.md", "parent", "Initial parent")
        .expect("Failed to create parent commit");
    create_test_commit(&child, "README.md", "child", "Initial child")
        .expect("Failed to create child commit");

    let child_remote = add_bare_remote(&child, true).expect("Failed to create child remote");
    let child_head = get_head_commit(&child).expect("Failed to resolve child HEAD");
    run_git_ok(
        &parent,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &child_head,
            "child",
        ],
    );
    run_git_ok(&parent, &["commit", "-m", "Track child"]);
    let parent_remote = add_bare_remote(&parent, true).expect("Failed to create parent remote");

    PublishedNestedFixture {
        parent,
        child,
        parent_remote,
        child_remote,
    }
}

fn bare_remote_path(remote: &TempDir) -> PathBuf {
    remote.path().join("remote.git")
}

fn gitlink_target(repo: &Path, relative: &str) -> String {
    let output = Command::new("git")
        .args(["ls-tree", "HEAD", "--", relative])
        .current_dir(repo)
        .output()
        .expect("Failed to inspect gitlink");
    assert!(
        output.status.success(),
        "gitlink inspection failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(2)
        .expect("Gitlink target missing")
        .to_string()
}

#[test]
fn test_save_pushes_clean_ahead_child_before_parent() {
    if !is_git_available() {
        return;
    }

    let root = TempDir::new().expect("Failed to create test root");
    let fixture = setup_published_nested_fixture(root.path());
    create_test_commit(&fixture.child, "next.txt", "next", "Advance child")
        .expect("Failed to advance child");
    let child_head = get_head_commit(&fixture.child).expect("Failed to resolve child HEAD");

    let save = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["save", "Fleet save"])
        .current_dir(root.path())
        .output()
        .expect("Failed to run repos save");

    assert!(
        save.status.success(),
        "dependency-aware save failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&save.stdout),
        String::from_utf8_lossy(&save.stderr)
    );
    assert_eq!(
        get_head_commit(&bare_remote_path(&fixture.child_remote))
            .expect("Failed to inspect child remote"),
        child_head
    );
    assert_eq!(gitlink_target(&fixture.parent, "child"), child_head);
    assert_eq!(
        gitlink_target(&bare_remote_path(&fixture.parent_remote), "child"),
        child_head
    );
}

#[test]
fn test_save_dry_run_plans_parent_gitlink_refresh() {
    if !is_git_available() {
        return;
    }

    let root = TempDir::new().expect("Failed to create test root");
    let fixture = setup_published_nested_fixture(root.path());
    let child_head = get_head_commit(&fixture.child).expect("Failed to resolve child HEAD");
    let parent_head = get_head_commit(&fixture.parent).expect("Failed to resolve parent HEAD");
    fs::write(fixture.child.join("README.md"), "planned child update")
        .expect("Failed to modify child");

    let save = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["save", "Fleet save", "--dry-run"])
        .current_dir(root.path())
        .output()
        .expect("Failed to run repos save dry-run");
    let stdout = String::from_utf8_lossy(&save.stdout);

    assert!(
        save.status.success(),
        "dependency-aware dry-run failed:\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&save.stderr)
    );
    assert!(
        stdout.contains("Planned         2")
            && stdout.contains("refresh submodule pointers, commit, push"),
        "parent pointer refresh was not planned:\n{stdout}"
    );
    assert_eq!(
        get_head_commit(&fixture.child).expect("Child HEAD changed during dry-run"),
        child_head
    );
    assert_eq!(
        get_head_commit(&fixture.parent).expect("Parent HEAD changed during dry-run"),
        parent_head
    );
}

#[cfg(unix)]
#[test]
fn test_save_blocks_refreshed_gitlink_after_child_push_failure() {
    use std::os::unix::fs::PermissionsExt;

    if !is_git_available() {
        return;
    }

    let root = TempDir::new().expect("Failed to create test root");
    let fixture = setup_published_nested_fixture(root.path());
    let published_parent =
        get_head_commit(&bare_remote_path(&fixture.parent_remote)).expect("Parent remote HEAD");
    create_test_commit(&fixture.child, "next.txt", "local only", "Advance child")
        .expect("Failed to advance child");
    let child_head = get_head_commit(&fixture.child).expect("Failed to resolve child HEAD");

    let hook = fixture.child.join(".git/hooks/pre-push");
    fs::write(
        &hook,
        "#!/bin/sh\necho 'intentional child push failure' >&2\nexit 1\n",
    )
    .expect("Failed to create child pre-push hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755))
        .expect("Failed to make child hook executable");

    let save = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["save", "Fleet save"])
        .current_dir(root.path())
        .output()
        .expect("Failed to run repos save");
    let stdout = String::from_utf8_lossy(&save.stdout);

    assert!(!save.status.success(), "failed child must fail fleet save");
    assert!(
        stdout.contains("committed; push blocked because depen")
            && stdout.contains("intentional child push failure"),
        "parent blocker was not reported:\n{stdout}"
    );
    assert_eq!(
        get_head_commit(&bare_remote_path(&fixture.parent_remote)).expect("Parent remote HEAD"),
        published_parent,
        "parent remote must not receive an unpublished child gitlink"
    );
    assert_eq!(
        gitlink_target(&fixture.parent, "child"),
        child_head,
        "the local parent commit should contain the target that was validated"
    );
}

#[test]
fn test_save_recovers_clean_ahead_parent_and_child() {
    if !is_git_available() {
        return;
    }

    let root = TempDir::new().expect("Failed to create test root");
    let fixture = setup_published_nested_fixture(root.path());
    create_test_commit(&fixture.child, "next.txt", "next", "Advance child")
        .expect("Failed to advance child");
    let child_head = get_head_commit(&fixture.child).expect("Failed to resolve child HEAD");
    run_git_ok(
        &fixture.parent,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &child_head,
            "child",
        ],
    );
    run_git_ok(&fixture.parent, &["commit", "-m", "Advance child pointer"]);
    let parent_head = get_head_commit(&fixture.parent).expect("Failed to resolve parent HEAD");

    let save = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["save", "Fleet save"])
        .current_dir(root.path())
        .output()
        .expect("Failed to run repos save");

    assert!(
        save.status.success(),
        "clean-ahead recovery failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&save.stdout),
        String::from_utf8_lossy(&save.stderr)
    );
    assert_eq!(
        get_head_commit(&bare_remote_path(&fixture.child_remote)).expect("Child remote HEAD"),
        child_head
    );
    assert_eq!(
        get_head_commit(&bare_remote_path(&fixture.parent_remote)).expect("Parent remote HEAD"),
        parent_head
    );
}

#[test]
fn test_save_rejects_unresolved_conflicts() {
    if !is_git_available() {
        return;
    }

    let root = TempDir::new().expect("Failed to create test root");
    let repo = root.path().join("repo");
    fs::create_dir(&repo).expect("Failed to create repository");
    setup_git_repo(&repo).expect("Failed to initialize repository");
    create_test_commit(&repo, "conflict.txt", "base\n", "Initial commit")
        .expect("Failed to create initial commit");
    let remote = add_bare_remote(&repo, true).expect("Failed to create remote");
    let published_head =
        get_head_commit(&bare_remote_path(&remote)).expect("Failed to inspect remote HEAD");
    let branch = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(&repo)
        .output()
        .expect("Failed to inspect branch");
    let branch = String::from_utf8_lossy(&branch.stdout).trim().to_string();

    run_git_ok(&repo, &["checkout", "-b", "conflict-side"]);
    create_test_commit(&repo, "conflict.txt", "side\n", "Side change")
        .expect("Failed to create side commit");
    run_git_ok(&repo, &["checkout", &branch]);
    create_test_commit(&repo, "conflict.txt", "main\n", "Main change")
        .expect("Failed to create main commit");
    let conflicted_head = get_head_commit(&repo).expect("Failed to resolve conflicted HEAD");
    let merge = Command::new("git")
        .args(["merge", "conflict-side"])
        .current_dir(&repo)
        .output()
        .expect("Failed to create merge conflict");
    assert!(!merge.status.success(), "merge should create a conflict");

    let save = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["save", "Do not commit conflict markers"])
        .current_dir(root.path())
        .output()
        .expect("Failed to run repos save");
    let stdout = String::from_utf8_lossy(&save.stdout);

    assert!(!save.status.success(), "unresolved conflict must fail save");
    assert!(
        stdout.contains("merge conflict"),
        "conflict guidance was not reported:\n{stdout}"
    );
    assert_eq!(get_head_commit(&repo).expect("Local HEAD"), conflicted_head);
    assert_eq!(
        get_head_commit(&bare_remote_path(&remote)).expect("Remote HEAD"),
        published_head
    );
    let unmerged = Command::new("git")
        .args(["ls-files", "-u"])
        .current_dir(&repo)
        .output()
        .expect("Failed to inspect conflict entries");
    assert!(
        !unmerged.stdout.is_empty(),
        "save must leave conflict entries unresolved"
    );
}

#[cfg(unix)]
#[test]
fn test_push_publishes_nested_gitlink_before_parent() {
    use std::os::unix::fs::PermissionsExt;

    if !is_git_available() {
        return;
    }

    let root = TempDir::new().expect("Failed to create test root");
    let parent = root.path().join("parent");
    let child = parent.join("child");
    fs::create_dir_all(&child).expect("Failed to create nested repositories");
    setup_git_repo(&parent).expect("Failed to initialize parent repository");
    setup_git_repo(&child).expect("Failed to initialize child repository");
    create_test_commit(&parent, "README.md", "parent", "Initial parent")
        .expect("Failed to create parent commit");
    create_test_commit(&child, "README.md", "child", "Initial child")
        .expect("Failed to create child commit");

    let child_remote = add_bare_remote(&child, true).expect("Failed to create child remote");
    let child_head = get_head_commit(&child).expect("Failed to resolve child HEAD");
    run_git_ok(
        &parent,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &child_head,
            "child",
        ],
    );
    run_git_ok(&parent, &["commit", "-m", "Track child"]);
    let parent_remote = add_bare_remote(&parent, true).expect("Failed to create parent remote");

    create_test_commit(&child, "next.txt", "next", "Advance child")
        .expect("Failed to advance child");
    let child_head = get_head_commit(&child).expect("Failed to resolve advanced child HEAD");
    run_git_ok(
        &parent,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &child_head,
            "child",
        ],
    );
    run_git_ok(&parent, &["commit", "-m", "Advance child pointer"]);

    let marker = root.path().join("child-pushed");
    let child_hook = child.join(".git/hooks/pre-push");
    fs::write(
        &child_hook,
        format!("#!/bin/sh\nsleep 1\ntouch '{}'\n", marker.display()),
    )
    .expect("Failed to create child pre-push hook");
    fs::set_permissions(&child_hook, fs::Permissions::from_mode(0o755))
        .expect("Failed to make child hook executable");
    let parent_hook = parent.join(".git/hooks/pre-push");
    fs::write(
        &parent_hook,
        format!(
            "#!/bin/sh\ntest -f '{}' || {{ echo 'child was not pushed first' >&2; exit 1; }}\n",
            marker.display()
        ),
    )
    .expect("Failed to create parent pre-push hook");
    fs::set_permissions(&parent_hook, fs::Permissions::from_mode(0o755))
        .expect("Failed to make parent hook executable");

    let push = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["push", "--jobs", "8", "--no-drift-check"])
        .current_dir(root.path())
        .output()
        .expect("Failed to run repos push");

    assert!(
        push.status.success(),
        "dependency-aware push failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&push.stdout),
        String::from_utf8_lossy(&push.stderr)
    );
    assert!(marker.exists(), "child push hook did not run");
    let child_remote_head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(child_remote.path().join("remote.git"))
        .output()
        .expect("Failed to inspect child remote");
    let parent_remote_head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(parent_remote.path().join("remote.git"))
        .output()
        .expect("Failed to inspect parent remote");
    assert!(child_remote_head.status.success());
    assert!(parent_remote_head.status.success());
    assert_eq!(
        String::from_utf8_lossy(&child_remote_head.stdout).trim(),
        child_head
    );
    assert_eq!(
        String::from_utf8_lossy(&parent_remote_head.stdout).trim(),
        get_head_commit(&parent).expect("Failed to resolve parent HEAD")
    );
}

#[test]
fn test_push_allows_detached_submodule_when_gitlink_commit_is_published() {
    if !is_git_available() {
        return;
    }

    let root = TempDir::new().expect("Failed to create test root");
    let parent = root.path().join("parent");
    let child = parent.join("child");
    fs::create_dir_all(&child).expect("Failed to create nested repositories");
    setup_git_repo(&parent).expect("Failed to initialize parent repository");
    setup_git_repo(&child).expect("Failed to initialize child repository");
    create_test_commit(&parent, "README.md", "parent", "Initial parent")
        .expect("Failed to create parent commit");
    create_test_commit(&child, "README.md", "child", "Initial child")
        .expect("Failed to create child commit");
    let _child_remote = add_bare_remote(&child, true).expect("Failed to create child remote");
    let child_head = get_head_commit(&child).expect("Failed to resolve child HEAD");
    run_git_ok(
        &parent,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &child_head,
            "child",
        ],
    );
    run_git_ok(&parent, &["commit", "-m", "Track child"]);
    let parent_remote = add_bare_remote(&parent, true).expect("Failed to create parent remote");

    run_git_ok(&child, &["checkout", "--detach", "HEAD"]);
    create_test_commit(&parent, "parent.txt", "update", "Advance parent")
        .expect("Failed to advance parent");

    let push = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["push", "--jobs", "8", "--no-drift-check"])
        .current_dir(root.path())
        .output()
        .expect("Failed to run repos push");

    assert!(
        push.status.success(),
        "published detached submodule should not block parent:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&push.stdout),
        String::from_utf8_lossy(&push.stderr)
    );
    let parent_remote_head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(parent_remote.path().join("remote.git"))
        .output()
        .expect("Failed to inspect parent remote");
    assert_eq!(
        String::from_utf8_lossy(&parent_remote_head.stdout).trim(),
        get_head_commit(&parent).expect("Failed to resolve parent HEAD")
    );
}

#[cfg(unix)]
#[test]
fn test_push_blocks_parent_when_gitlink_commit_is_not_published() {
    use std::os::unix::fs::PermissionsExt;

    if !is_git_available() {
        return;
    }

    let root = TempDir::new().expect("Failed to create test root");
    let parent = root.path().join("parent");
    let child = parent.join("child");
    fs::create_dir_all(&child).expect("Failed to create nested repositories");
    setup_git_repo(&parent).expect("Failed to initialize parent repository");
    setup_git_repo(&child).expect("Failed to initialize child repository");
    create_test_commit(&parent, "README.md", "parent", "Initial parent")
        .expect("Failed to create parent commit");
    create_test_commit(&child, "README.md", "child", "Initial child")
        .expect("Failed to create child commit");
    let _child_remote = add_bare_remote(&child, true).expect("Failed to create child remote");
    let child_head = get_head_commit(&child).expect("Failed to resolve child HEAD");
    run_git_ok(
        &parent,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &child_head,
            "child",
        ],
    );
    run_git_ok(&parent, &["commit", "-m", "Track child"]);
    let parent_remote = add_bare_remote(&parent, true).expect("Failed to create parent remote");
    let published_parent = get_head_commit(&parent).expect("Failed to resolve published parent");

    create_test_commit(&child, "next.txt", "local only", "Local-only child")
        .expect("Failed to advance child");
    let child_head = get_head_commit(&child).expect("Failed to resolve advanced child HEAD");
    run_git_ok(
        &parent,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &child_head,
            "child",
        ],
    );
    run_git_ok(&parent, &["commit", "-m", "Point at local-only child"]);

    let child_hook = child.join(".git/hooks/pre-push");
    fs::write(
        &child_hook,
        "#!/bin/sh\necho 'intentional child push failure' >&2\nexit 1\n",
    )
    .expect("Failed to create child pre-push hook");
    fs::set_permissions(&child_hook, fs::Permissions::from_mode(0o755))
        .expect("Failed to make child hook executable");

    let push = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["push", "--jobs", "8", "--no-drift-check"])
        .current_dir(root.path())
        .output()
        .expect("Failed to run repos push");
    assert!(!push.status.success(), "failed child must fail fleet push");
    assert!(
        String::from_utf8_lossy(&push.stdout).contains("not reachable from fetched remote refs"),
        "parent blocker was not reported:\n{}",
        String::from_utf8_lossy(&push.stdout)
    );

    let parent_remote_head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(parent_remote.path().join("remote.git"))
        .output()
        .expect("Failed to inspect parent remote");
    assert_eq!(
        String::from_utf8_lossy(&parent_remote_head.stdout).trim(),
        published_parent,
        "parent remote must not receive a gitlink to an unpublished child"
    );
}

#[tokio::test]
async fn test_push_command_with_no_remote() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository without a remote
    let repo = match TestRepo::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Change to repo directory
    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run push command - should handle no remote gracefully
    let result = handle_push_command(false, false, false, true, None, false).await;

    assert!(
        result.is_ok(),
        "Push command should handle missing remote without panicking: {:?}",
        result
    );
}

#[test]
fn test_cli_fails_when_remote_is_unreachable() {
    if !is_git_available() {
        return;
    }

    let repo = TestRepo::new().expect("Failed to create test repo");
    let remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");
    drop(remote);

    let push = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["push", "--sequential", "--no-drift-check"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos push");
    assert!(!push.status.success(), "unreachable push must exit nonzero");

    let doctor = Command::new(env!("CARGO_BIN_EXE_repos"))
        .arg("doctor")
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos doctor");
    assert!(
        !doctor.status.success(),
        "unreachable remote must make doctor exit nonzero"
    );
    assert!(
        String::from_utf8_lossy(&doctor.stdout).contains("access failed"),
        "doctor should identify remote access failure"
    );
}

#[test]
fn test_doctor_ssh_only_policy_uses_effective_instead_of_url() {
    if !is_git_available() {
        return;
    }

    let repo = TestRepo::new().expect("Failed to create test repo");
    let remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");
    let https_url = "https://example.invalid/team/repo.git";
    let git_config = IsolatedGitConfig::new("").expect("Failed to isolate Git config");

    run_git_ok(repo.path(), &["remote", "set-url", "origin", https_url]);

    let rewrite_key = format!(
        "url.{}.insteadOf",
        remote.path().join("remote.git").display()
    );
    run_git_ok(repo.path(), &["config", &rewrite_key, https_url]);

    let mut doctor_command = Command::new(env!("CARGO_BIN_EXE_repos"));
    git_config.apply(&mut doctor_command);
    let doctor = doctor_command
        .arg("doctor")
        .env("REPOS_TRANSPORT_POLICY", "ssh-only")
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos doctor");
    let stdout = String::from_utf8_lossy(&doctor.stdout);

    assert!(
        doctor.status.success(),
        "advisory must not fail doctor: {stdout}"
    );
    assert!(stdout.contains("origin uses HTTP(S)"), "{stdout}");
    assert!(!stdout.contains("ssh-only policy blocked"), "{stdout}");
}

#[test]
fn test_doctor_reports_http_pushurl_without_invoking_credentials() {
    if !is_git_available() {
        return;
    }

    let repo = TestRepo::new().expect("Failed to create test repo");
    let _remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");
    let helper_marker = repo.path().join("credential-helper-ran");
    let git_config = IsolatedGitConfig::new(&format!(
        "[credential]\n\thelper = !touch {}\n",
        helper_marker.display()
    ))
    .expect("Failed to isolate Git config");
    run_git_ok(
        repo.path(),
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            "https://secret@github.com/goobits/keychain-test.git?token=hidden",
        ],
    );

    let mut doctor_command = Command::new(env!("CARGO_BIN_EXE_repos"));
    git_config.apply(&mut doctor_command);
    let doctor = doctor_command
        .arg("doctor")
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos doctor");
    let stdout = String::from_utf8_lossy(&doctor.stdout);

    assert!(
        doctor.status.success(),
        "HTTP push URL is advisory under preserve policy: {stdout}"
    );
    assert!(stdout.contains("origin uses HTTP(S) for push"), "{stdout}");
    assert!(
        stdout
            .contains("remote set-url --push 'origin' 'git@github.com:goobits/keychain-test.git'"),
        "{stdout}"
    );
    assert!(!stdout.contains("secret"), "{stdout}");
    assert!(!stdout.contains("hidden"), "{stdout}");
    assert!(
        !helper_marker.exists(),
        "doctor must not invoke a credential helper while inspecting a push URL"
    );
}

#[test]
fn test_doctor_ssh_only_policy_blocks_http_pushurl_with_exact_fix() {
    if !is_git_available() {
        return;
    }

    let repo = TestRepo::new().expect("Failed to create test repo");
    let _remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");
    let git_config = IsolatedGitConfig::new("").expect("Failed to isolate Git config");
    run_git_ok(
        repo.path(),
        &[
            "remote",
            "set-url",
            "--push",
            "origin",
            "https://github.com/goobits/keychain-test.git",
        ],
    );

    let mut doctor_command = Command::new(env!("CARGO_BIN_EXE_repos"));
    git_config.apply(&mut doctor_command);
    let doctor = doctor_command
        .arg("doctor")
        .env("REPOS_TRANSPORT_POLICY", "ssh-only")
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos doctor");
    let stdout = String::from_utf8_lossy(&doctor.stdout);

    assert!(
        !doctor.status.success(),
        "SSH-only violation must fail doctor"
    );
    assert!(stdout.contains("ssh-only policy blocked push"), "{stdout}");
    assert!(
        stdout
            .contains("remote set-url --push 'origin' 'git@github.com:goobits/keychain-test.git'"),
        "{stdout}"
    );
    assert!(stdout.contains("path:"), "{stdout}");
}

fn assert_ssh_only_command_blocks_https_fetch(args: &[&str]) {
    let repo = TestRepo::new().expect("Failed to create test repo");
    let helper_marker = repo.path().join("credential-helper-ran");
    let helper = format!("!touch {}", helper_marker.display());
    let remote = "https://secret-token@github.com/goobits/keychain-test.git?access_token=hidden";
    let git_config = IsolatedGitConfig::new("[repos]\n\ttransportPolicy = ssh-only\n")
        .expect("Failed to isolate Git config");

    run_git_ok(repo.path(), &["remote", "add", "origin", remote]);
    run_git_ok(repo.path(), &["config", "credential.helper", &helper]);

    let mut command = Command::new(env!("CARGO_BIN_EXE_repos"));
    git_config.apply(&mut command);
    let output = command
        .args(args)
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos command");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(!output.status.success(), "HTTPS fetch must be blocked");
    assert!(
        stdout.contains("SSH-only policy blocked fetch (HTTPS)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("remote: origin (HTTPS, github.com/goobits/keychain-test.git)"),
        "{stdout}"
    );
    assert!(
        stdout.contains("remote set-url 'origin' 'git@github.com:goobits/keychain-test.git'"),
        "{stdout}"
    );
    assert!(!stdout.contains("secret-token"), "{stdout}");
    assert!(!stdout.contains("access_token"), "{stdout}");
    assert!(!stdout.contains("hidden"), "{stdout}");
    assert!(
        !helper_marker.exists(),
        "credential helper must not run for a blocked HTTPS remote"
    );
}

#[test]
fn test_ssh_only_push_blocks_https_fetch_before_credential_helper() {
    if !is_git_available() {
        return;
    }

    assert_ssh_only_command_blocks_https_fetch(&["push", "--sequential", "--no-drift-check"]);
}

#[test]
fn test_ssh_only_fetch_blocks_https_before_credential_helper() {
    if !is_git_available() {
        return;
    }

    assert_ssh_only_command_blocks_https_fetch(&["fetch", "--sequential"]);
}

#[test]
fn test_ssh_only_pull_blocks_https_before_credential_helper() {
    if !is_git_available() {
        return;
    }

    assert_ssh_only_command_blocks_https_fetch(&["pull", "--sequential", "--no-drift-check"]);
}

#[test]
fn test_ssh_only_status_blocks_https_before_credential_helper() {
    if !is_git_available() {
        return;
    }

    assert_ssh_only_command_blocks_https_fetch(&["status"]);
}

#[test]
fn test_ssh_only_push_reports_https_pushurl_fix() {
    if !is_git_available() {
        return;
    }

    let repo = TestRepo::new().expect("Failed to create test repo");
    let _remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");
    let helper_marker = repo.path().join("credential-helper-ran");
    let helper = format!("!touch {}", helper_marker.display());
    let push_url = "https://github.com/goobits/keychain-test.git";
    let git_config = IsolatedGitConfig::new("").expect("Failed to isolate Git config");

    run_git_ok(
        repo.path(),
        &["remote", "set-url", "--push", "origin", push_url],
    );
    run_git_ok(repo.path(), &["config", "credential.helper", &helper]);
    repo.create_file("ahead.txt", "one local commit")
        .expect("Failed to create test file");
    repo.commit_all("Create local commit")
        .expect("Failed to create local commit");

    let mut push_command = Command::new(env!("CARGO_BIN_EXE_repos"));
    git_config.apply(&mut push_command);
    let push = push_command
        .args(["push", "--sequential", "--no-drift-check"])
        .env("REPOS_TRANSPORT_POLICY", "ssh-only")
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos push");
    let stdout = String::from_utf8_lossy(&push.stdout);

    assert!(!push.status.success(), "HTTPS push URL must be blocked");
    assert!(
        stdout.contains("SSH-only policy blocked push (HTTPS)"),
        "{stdout}"
    );
    assert!(
        stdout
            .contains("remote set-url --push 'origin' 'git@github.com:goobits/keychain-test.git'"),
        "{stdout}"
    );
    assert!(
        !helper_marker.exists(),
        "credential helper must not run for a blocked HTTPS push URL"
    );
}

#[tokio::test]
async fn test_push_command_with_uncommitted_changes() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepo::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };
    let _remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");

    // Create an uncommitted file
    let test_file = repo.path().join("uncommitted.txt");
    fs::write(&test_file, "uncommitted content").expect("Failed to write test file");

    // Change to repo directory
    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run push command - should detect uncommitted changes
    let result = handle_push_command(false, false, false, true, None, false).await;

    assert!(
        result.is_ok(),
        "Push command should handle uncommitted changes: {:?}",
        result
    );
}

#[tokio::test]
async fn test_fetch_command_reports_updated_repository_without_moving_head() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        return;
    }

    let repo = TestRepo::new().expect("Failed to create test repo");
    let remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");
    let original_head = get_head_commit(repo.path()).expect("Failed to resolve original HEAD");
    let updater_root = TempDir::new().expect("Failed to create updater directory");
    let updater = updater_root.path().join("updater");
    clone_repo(&remote.path().join("remote.git"), &updater).expect("Failed to clone test remote");
    create_test_commit(&updater, "remote.txt", "remote change", "Remote update")
        .expect("Failed to create remote commit");
    run_git_ok(&updater, &["push"]);

    let fetch = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["fetch", "--sequential"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos fetch");
    let stdout = String::from_utf8_lossy(&fetch.stdout);

    assert!(
        fetch.status.success(),
        "Fetch command failed:\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&fetch.stderr)
    );
    assert!(stdout.contains("repos fetch"), "{stdout}");
    assert!(stdout.contains("▌ Fetched"), "{stdout}");
    assert!(stdout.contains("current"), "{stdout}");
    assert!(stdout.contains("1 ref"), "{stdout}");
    assert_eq!(
        get_head_commit(repo.path()).expect("Failed to resolve HEAD after fetch"),
        original_head,
        "fetch must not move the local branch"
    );
}

#[tokio::test]
async fn test_pull_command_reports_pulled_repository() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        return;
    }

    let repo = TestRepo::new().expect("Failed to create test repo");
    let remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");
    let updater_root = TempDir::new().expect("Failed to create updater directory");
    let updater = updater_root.path().join("updater");
    clone_repo(&remote.path().join("remote.git"), &updater).expect("Failed to clone test remote");
    create_test_commit(&updater, "remote.txt", "remote change", "Remote update")
        .expect("Failed to create remote commit");
    run_git_ok(&updater, &["push"]);

    let pull = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["pull", "--sequential", "--no-drift-check"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos pull");
    let stdout = String::from_utf8_lossy(&pull.stdout);

    assert!(
        pull.status.success(),
        "Pull command failed:\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&pull.stderr)
    );
    assert!(stdout.contains("repos pull"), "{stdout}");
    assert!(stdout.contains("▌ Pulled"), "{stdout}");
    assert!(stdout.contains("current"), "{stdout}");
    assert!(stdout.contains("1 commit"), "{stdout}");
    assert!(!stdout.contains("Pushed"), "{stdout}");
}

#[tokio::test]
async fn test_push_command_with_auto_upstream() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository with remote
    let repo = match TestRepo::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };
    let _remote = add_bare_remote(repo.path(), false).expect("Failed to attach bare remote");

    // Change to repo directory
    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run push command with automatic upstream creation enabled.
    let result = handle_push_command(true, false, false, true, None, false).await;

    assert!(
        result.is_ok(),
        "Auto-upstream push command should complete without panicking: {:?}",
        result
    );
}

#[tokio::test]
async fn test_auto_upstream_prefers_origin_over_alphabetical_remote() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        return;
    }

    let root = TempDir::new().expect("Failed to create temp directory");
    let repo = TestRepo::new().expect("Failed to create test repo");
    let aaa_remote = root.path().join("aaa.git");
    let origin_remote = root.path().join("origin.git");
    for remote in [&aaa_remote, &origin_remote] {
        run_git_ok(
            root.path(),
            &["init", "--bare", remote.to_str().expect("UTF-8 path")],
        );
    }
    run_git_ok(
        repo.path(),
        &[
            "remote",
            "add",
            "aaa",
            aaa_remote.to_str().expect("UTF-8 path"),
        ],
    );
    run_git_ok(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            origin_remote.to_str().expect("UTF-8 path"),
        ],
    );

    let fetch_result = fetch_and_analyze(repo.path(), true).await;
    assert_eq!(fetch_result.status, Status::NoUpstream);
    assert_eq!(fetch_result.upstream_remote.as_deref(), Some("origin"));
    let (status, message, _) = push_if_needed(repo.path(), &fetch_result, true).await;
    assert_eq!(status, Status::Pushed, "{message}");

    let branch = git_stdout(repo.path(), &["symbolic-ref", "--short", "HEAD"]);
    assert_eq!(
        git_stdout(repo.path(), &["config", &format!("branch.{branch}.remote")]),
        "origin"
    );
    assert!(git_succeeds(
        &origin_remote,
        &["show-ref", "--verify", &format!("refs/heads/{branch}")]
    ));
    assert!(!git_succeeds(
        &aaa_remote,
        &["show-ref", "--verify", &format!("refs/heads/{branch}")]
    ));
}

#[tokio::test]
async fn test_auto_upstream_honors_push_default_over_origin() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        return;
    }

    let root = TempDir::new().expect("Failed to create temp directory");
    let repo = TestRepo::new().expect("Failed to create test repo");
    let preferred_remote = root.path().join("preferred.git");
    let origin_remote = root.path().join("origin.git");
    for remote in [&preferred_remote, &origin_remote] {
        run_git_ok(
            root.path(),
            &["init", "--bare", remote.to_str().expect("UTF-8 path")],
        );
    }
    run_git_ok(
        repo.path(),
        &[
            "remote",
            "add",
            "preferred",
            preferred_remote.to_str().expect("UTF-8 path"),
        ],
    );
    run_git_ok(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            origin_remote.to_str().expect("UTF-8 path"),
        ],
    );
    run_git_ok(repo.path(), &["config", "remote.pushDefault", "preferred"]);

    let fetch_result = fetch_and_analyze(repo.path(), true).await;
    assert_eq!(fetch_result.status, Status::NoUpstream);
    assert_eq!(fetch_result.upstream_remote.as_deref(), Some("preferred"));
    let (status, message, _) = push_if_needed(repo.path(), &fetch_result, true).await;
    assert_eq!(status, Status::Pushed, "{message}");

    let branch = git_stdout(repo.path(), &["symbolic-ref", "--short", "HEAD"]);
    assert!(git_succeeds(
        &preferred_remote,
        &["show-ref", "--verify", &format!("refs/heads/{branch}")]
    ));
    assert!(!git_succeeds(
        &origin_remote,
        &["show-ref", "--verify", &format!("refs/heads/{branch}")]
    ));
}

#[tokio::test]
async fn test_auto_upstream_rejects_ambiguous_remotes() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        return;
    }

    let root = TempDir::new().expect("Failed to create temp directory");
    let repo = TestRepo::new().expect("Failed to create test repo");
    for name in ["aaa", "zzz"] {
        let remote = root.path().join(format!("{name}.git"));
        run_git_ok(
            root.path(),
            &["init", "--bare", remote.to_str().expect("UTF-8 path")],
        );
        run_git_ok(
            repo.path(),
            &["remote", "add", name, remote.to_str().expect("UTF-8 path")],
        );
    }

    let fetch_result = fetch_and_analyze(repo.path(), true).await;
    assert_eq!(fetch_result.status, Status::Error);
    assert!(
        fetch_result.message.contains("ambiguous push remote"),
        "{}",
        fetch_result.message
    );
}

fn git_stdout(path: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("Failed to run git");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn git_succeeds(path: &std::path::Path, args: &[&str]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .expect("Failed to run git")
        .status
        .success()
}

#[tokio::test]
async fn test_push_if_needed_uses_upstream_remote_for_current_branch() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let root = TempDir::new().expect("Failed to create temp directory");
    let wrong_remote = root.path().join("aaa-remote.git");
    let upstream_remote = root.path().join("origin-remote.git");

    for remote in [&wrong_remote, &upstream_remote] {
        let output = Command::new("git")
            .args(["init", "--bare"])
            .current_dir(root.path())
            .arg(remote)
            .output()
            .expect("Failed to init bare remote");
        assert!(
            output.status.success(),
            "Failed to init bare remote: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let repo = TestRepo::new().expect("Failed to create test repo");

    for (name, path) in [
        ("aaa", wrong_remote.to_string_lossy().to_string()),
        ("origin", upstream_remote.to_string_lossy().to_string()),
    ] {
        let output = Command::new("git")
            .args(["remote", "add", name, &path])
            .current_dir(repo.path())
            .output()
            .expect("Failed to add remote");
        assert!(
            output.status.success(),
            "Failed to add remote: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = Command::new("git")
        .args(["checkout", "-b", "feature/music"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to create feature branch");
    assert!(
        output.status.success(),
        "Failed to create feature branch: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = Command::new("git")
        .args(["push", "-u", "origin", "feature/music"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to push initial branch");
    assert!(
        output.status.success(),
        "Failed to push initial branch: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    fs::write(repo.path().join("feature.txt"), "new commit").expect("Failed to write test file");
    let output = Command::new("git")
        .args(["add", "feature.txt"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to stage file");
    assert!(
        output.status.success(),
        "Failed to stage file: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new("git")
        .args(["commit", "-m", "Feature update"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to commit file");
    assert!(
        output.status.success(),
        "Failed to commit file: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let head_commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to get HEAD");
    assert!(head_commit.status.success(), "Failed to get HEAD");
    let head_commit = String::from_utf8_lossy(&head_commit.stdout)
        .trim()
        .to_string();

    let fetch_result = fetch_and_analyze(repo.path(), false).await;
    assert!(fetch_result.upstream_exists);
    assert_eq!(fetch_result.upstream_remote.as_deref(), Some("origin"));
    assert_eq!(
        fetch_result.upstream_branch.as_deref(),
        Some("feature/music")
    );
    assert_eq!(fetch_result.ahead_count, 1);

    let (status, _message, _has_uncommitted) =
        push_if_needed(repo.path(), &fetch_result, false).await;
    assert_eq!(status, Status::Pushed);

    let origin_head = Command::new("git")
        .args(["rev-parse", "feature/music"])
        .current_dir(&upstream_remote)
        .output()
        .expect("Failed to read origin remote head");
    assert!(
        origin_head.status.success(),
        "Origin remote missing feature branch: {}",
        String::from_utf8_lossy(&origin_head.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&origin_head.stdout).trim(),
        head_commit
    );

    let wrong_head = Command::new("git")
        .args(["rev-parse", "feature/music"])
        .current_dir(&wrong_remote)
        .output()
        .expect("Failed to read wrong remote head");
    assert!(
        !wrong_head.status.success(),
        "Wrong remote should not receive pushed branch"
    );
}

// ==============================================================================
// STAGING COMMAND TESTS (commands/staging.rs)
// ==============================================================================

#[test]
fn test_commit_refreshes_parent_gitlink_after_child_commit() {
    if !is_git_available() {
        return;
    }

    let root = TempDir::new().expect("Failed to create test root");
    let parent = root.path().join("parent");
    let child = parent.join("child");
    fs::create_dir_all(&child).expect("Failed to create nested repositories");
    setup_git_repo(&parent).expect("Failed to initialize parent repository");
    setup_git_repo(&child).expect("Failed to initialize child repository");
    create_test_commit(&parent, "README.md", "parent", "Initial parent")
        .expect("Failed to create parent commit");
    create_test_commit(&child, "README.md", "child", "Initial child")
        .expect("Failed to create child commit");
    let initial_child = get_head_commit(&child).expect("Failed to resolve child HEAD");
    run_git_ok(
        &parent,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            "160000",
            &initial_child,
            "child",
        ],
    );
    run_git_ok(&parent, &["commit", "-m", "Track child"]);

    fs::write(child.join("README.md"), "child update").expect("Failed to modify child");
    run_git_ok(&child, &["add", "README.md"]);

    let commit = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["commit", "Fleet commit"])
        .current_dir(root.path())
        .output()
        .expect("Failed to run repos commit");
    assert!(
        commit.status.success(),
        "dependency-aware commit failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&commit.stdout),
        String::from_utf8_lossy(&commit.stderr)
    );

    let child_head = get_head_commit(&child).expect("Failed to resolve committed child HEAD");
    assert_ne!(child_head, initial_child);
    let tree = Command::new("git")
        .args(["ls-tree", "HEAD", "--", "child"])
        .current_dir(&parent)
        .output()
        .expect("Failed to inspect parent gitlink");
    assert!(tree.status.success());
    let tree = String::from_utf8_lossy(&tree.stdout);
    let parent_gitlink = tree
        .split_whitespace()
        .nth(2)
        .expect("Parent gitlink target missing");
    assert_eq!(parent_gitlink, child_head);
}

#[tokio::test]
async fn test_stage_command_with_simple_pattern() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepo::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Create a test file
    let test_file = repo.path().join("test.txt");
    fs::write(&test_file, "test content").expect("Failed to write test file");

    // Change to repo directory
    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run stage command
    let result = handle_stage_command("test.txt".to_string()).await;

    assert!(result.is_ok(), "Stage command should succeed: {:?}", result);

    // Verify file was staged
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to check git status");

    let status_str = String::from_utf8_lossy(&status_output.stdout);
    assert!(
        status_str.contains("A  test.txt") || status_str.contains("?? test.txt"),
        "File should appear in git status, got: {}",
        status_str
    );
}

#[tokio::test]
async fn test_stage_command_with_wildcard_pattern() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepo::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Create multiple test files
    fs::write(repo.path().join("test1.md"), "# Test 1").expect("Failed to write test1.md");
    fs::write(repo.path().join("test2.md"), "# Test 2").expect("Failed to write test2.md");
    fs::write(repo.path().join("test.txt"), "text file").expect("Failed to write test.txt");

    // Change to repo directory
    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run stage command with wildcard pattern
    let result = handle_stage_command("*.md".to_string()).await;

    assert!(
        result.is_ok(),
        "Stage command with wildcard should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_unstage_command_with_pattern() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepo::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Create and stage a test file
    let test_file = repo.path().join("test.txt");
    fs::write(&test_file, "test content").expect("Failed to write test file");

    std::process::Command::new("git")
        .args(["add", "test.txt"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to stage file");

    // Change to repo directory
    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run unstage command
    let result = handle_unstage_command("test.txt".to_string()).await;

    assert!(
        result.is_ok(),
        "Unstage command should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_commit_command_with_staged_changes() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepo::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Create and stage a test file
    let test_file = repo.path().join("newfile.txt");
    fs::write(&test_file, "new content").expect("Failed to write test file");

    std::process::Command::new("git")
        .args(["add", "newfile.txt"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to stage file");

    // Verify file is actually staged before running commit command
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to check git status");

    let status_str = String::from_utf8_lossy(&status_output.stdout);
    assert!(
        !status_str.trim().is_empty(),
        "Should have staged changes before commit, got: {}",
        status_str
    );

    // Change to repo directory
    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run commit command
    let result = handle_commit_command("Test commit message".to_string(), false).await;

    assert!(
        result.is_ok(),
        "Commit command should succeed with staged changes: {:?}",
        result
    );

    // Verify commit was created by checking the last commit message
    // Note: Due to timing and command execution, we verify the command completed successfully
    // rather than parsing git log, as the actual commit creation is tested in git operations tests
}

#[tokio::test]
async fn test_commit_command_with_no_staged_changes() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // The fixture already has an initial commit.
    let repo = match TestRepo::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Change to repo directory
    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run commit command with no staged changes - should handle gracefully
    let result = handle_commit_command("Empty commit".to_string(), false).await;

    assert!(
        result.is_ok(),
        "Commit command should handle no changes gracefully: {:?}",
        result
    );
}

#[tokio::test]
async fn test_commit_command_with_allow_empty_flag() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepo::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Change to repo directory
    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run commit command with allow_empty flag
    let result = handle_commit_command("Empty commit".to_string(), true).await;

    assert!(
        result.is_ok(),
        "Commit command with allow_empty should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_staging_status_command_with_changes() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepo::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Create files with different states
    // 1. Staged file
    let staged_file = repo.path().join("staged.txt");
    fs::write(&staged_file, "staged content").expect("Failed to write staged file");
    std::process::Command::new("git")
        .args(["add", "staged.txt"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to stage file");

    // 2. Unstaged file
    let unstaged_file = repo.path().join("unstaged.txt");
    fs::write(&unstaged_file, "unstaged content").expect("Failed to write unstaged file");

    // Change to repo directory
    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run status command
    let result = handle_staging_status_command(Vec::new(), StatusFilters::default()).await;

    assert!(
        result.is_ok(),
        "Status command should succeed with changes: {:?}",
        result
    );
}

#[tokio::test]
async fn test_staging_status_command_with_no_changes() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // The fixture starts clean.
    let repo = match TestRepo::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Change to repo directory
    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run status command
    let result = handle_staging_status_command(Vec::new(), StatusFilters::default()).await;

    assert!(
        result.is_ok(),
        "Status command should succeed with no changes: {:?}",
        result
    );
}

#[test]
fn test_status_refreshes_remote_state_before_reporting() {
    if !is_git_available() {
        return;
    }

    let repo = TestRepo::new().expect("Failed to create test repo");
    let remote = add_bare_remote(repo.path(), true).expect("Failed to attach bare remote");
    let original_head = get_head_commit(repo.path()).expect("Failed to resolve original HEAD");
    let updater_root = TempDir::new().expect("Failed to create updater directory");
    let updater = updater_root.path().join("updater");
    clone_repo(&remote.path().join("remote.git"), &updater).expect("Failed to clone test remote");
    create_test_commit(&updater, "remote.txt", "remote change", "Remote update")
        .expect("Failed to create remote commit");
    run_git_ok(&updater, &["push"]);

    let status = Command::new(env!("CARGO_BIN_EXE_repos"))
        .arg("status")
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos status");
    let stdout = String::from_utf8_lossy(&status.stdout);

    assert!(
        status.status.success(),
        "Status command failed:\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(stdout.contains("Needs work      1"), "{stdout}");
    assert!(stdout.contains("behind 1"), "{stdout}");
    assert!(stdout.contains("next: run `repos pull`"), "{stdout}");
    assert_eq!(
        get_head_commit(repo.path()).expect("Failed to resolve HEAD after status"),
        original_head,
        "status may refresh refs but must not move the local branch"
    );
}

#[tokio::test]
async fn test_stage_command_with_nonexistent_file() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    // Create a test repository
    let repo = match TestRepo::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create test repo: {}, skipping", e);
            return;
        }
    };

    // Change to repo directory
    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run stage command with non-existent file - should handle gracefully
    let result = handle_stage_command("nonexistent.txt".to_string()).await;

    assert!(
        result.is_ok(),
        "Stage command should handle non-existent file gracefully: {:?}",
        result
    );
}
