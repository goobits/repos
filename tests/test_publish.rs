use goobits_repos::commands::publish::handle_publish_command;
use std::fs;

mod common;
use common::fixtures::TestRepo;
use common::git::is_git_available;
use common::CurrentDirGuard;

#[tokio::test]
async fn test_publish_rejects_missing_requested_repository() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        return;
    }

    let repo = TestRepo::new().expect("Failed to create test repo");
    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change directory");

    let error = handle_publish_command(
        vec!["missing-repository".to_string()],
        true,
        false,
        false,
        true,
        false,
    )
    .await
    .expect_err("missing explicit target must fail");

    assert!(
        error
            .to_string()
            .contains("requested publish repositories not found: missing-repository"),
        "{error}"
    );
}

#[tokio::test]
async fn test_publish_dry_run_cargo() {
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

    // Add Cargo.toml
    let cargo_toml = r#"[package]
name = "test-pkg"
version = "0.1.0"
"#;
    fs::write(repo.path().join("Cargo.toml"), cargo_toml).expect("Failed to write Cargo.toml");

    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run publish command with dry_run = true
    let result = handle_publish_command(
        vec![], // target_repos
        true,   // dry_run
        false,  // tag
        false,  // allow_dirty
        true,   // all (to ignore visibility check since test repo might be private/unknown)
        false,  // private_only
    )
    .await;

    assert!(
        result.is_ok(),
        "Publish dry-run should succeed: {:?}",
        result
    );
}

#[tokio::test]
async fn test_publish_dry_run_npm() {
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

    // Add package.json
    let package_json = r#"{
  "name": "test-pkg",
  "version": "0.1.0"
}"#;
    fs::write(repo.path().join("package.json"), package_json)
        .expect("Failed to write package.json");

    let _cwd = CurrentDirGuard::enter(repo.path()).expect("Failed to change dir");

    // Run publish command with dry_run = true
    let result = handle_publish_command(
        vec![], // target_repos
        true,   // dry_run
        false,  // tag
        false,  // allow_dirty
        true,   // all
        false,  // private_only
    )
    .await;

    assert!(
        result.is_ok(),
        "Publish dry-run should succeed: {:?}",
        result
    );
}
