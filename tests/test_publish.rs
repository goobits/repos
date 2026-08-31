use std::fs;
use std::process::Command;

mod common;
use common::fixtures::TestRepo;
use common::git::is_git_available;

#[tokio::test]
async fn test_publish_rejects_missing_requested_repository() {
    let _lock = common::lock_test().await;
    if !is_git_available() {
        return;
    }

    let repo = TestRepo::new().expect("Failed to create test repo");
    let output = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["publish", "--dry-run", "--all", "missing-repository"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos publish");
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "missing explicit target must fail"
    );
    assert!(
        stderr.contains("requested publish repositories not found: missing-repository"),
        "{stderr}"
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

    let result = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["publish", "--dry-run", "--all"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos publish");

    assert!(
        result.status.success(),
        "Publish dry-run should succeed: {}",
        String::from_utf8_lossy(&result.stderr)
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

    let result = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["publish", "--dry-run", "--all"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to run repos publish");

    assert!(
        result.status.success(),
        "Publish dry-run should succeed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
