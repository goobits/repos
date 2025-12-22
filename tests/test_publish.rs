use goobits_repos::commands::publish::handle_publish_command;
use std::env;
use std::fs;

mod common;
use common::{is_git_available, TestRepoBuilder};

#[tokio::test]
async fn test_publish_dry_run_cargo() {
    let _lock = common::lock_test();
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let original_dir = env::current_dir().expect("Failed to get current dir");

    // Create a test repository
    let repo = match TestRepoBuilder::new("test-publish-cargo").build() {
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

    // Change to repo directory
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run publish command with dry_run = true
    let result = handle_publish_command(
        vec![],     // target_repos
        true,       // dry_run
        false,      // tag
        false,      // allow_dirty
        true,       // all (to ignore visibility check since test repo might be private/unknown)
        false,      // public_only
        false,      // private_only
    ).await;

    // Restore original directory
    let _ = env::set_current_dir(&original_dir);

    assert!(result.is_ok(), "Publish dry-run should succeed: {:?}", result);
}

#[tokio::test]
async fn test_publish_dry_run_npm() {
    let _lock = common::lock_test();
    if !is_git_available() {
        eprintln!("Git not available, skipping test");
        return;
    }

    let original_dir = env::current_dir().expect("Failed to get current dir");

    // Create a test repository
    let repo = match TestRepoBuilder::new("test-publish-npm").build() {
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
    fs::write(repo.path().join("package.json"), package_json).expect("Failed to write package.json");

    // Change to repo directory
    env::set_current_dir(repo.path()).expect("Failed to change dir");

    // Run publish command with dry_run = true
    let result = handle_publish_command(
        vec![],     // target_repos
        true,       // dry_run
        false,      // tag
        false,      // allow_dirty
        true,       // all
        false,      // public_only
        false,      // private_only
    ).await;

    // Restore original directory
    let _ = env::set_current_dir(&original_dir);

    assert!(result.is_ok(), "Publish dry-run should succeed: {:?}", result);
}
