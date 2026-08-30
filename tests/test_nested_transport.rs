use std::process::Command;

use tempfile::TempDir;

mod common;
use common::git::{
    create_test_commit, is_git_available, run_git_ok, setup_git_repo, IsolatedGitConfig,
};

#[test]
fn nested_update_blocks_https_before_credentials_run() {
    if !is_git_available() {
        return;
    }

    let root = TempDir::new().expect("Failed to create test directory");
    let parent = root.path().join("parent");
    std::fs::create_dir(&parent).expect("Failed to create parent repository");
    setup_git_repo(&parent).expect("Failed to initialize parent repository");
    create_test_commit(&parent, "README.md", "parent", "Initial parent")
        .expect("Failed to commit parent repository");

    let nested = parent.join("shared");
    std::fs::create_dir(&nested).expect("Failed to create nested repository");
    setup_git_repo(&nested).expect("Failed to initialize nested repository");
    create_test_commit(&nested, "shared.txt", "nested", "Initial nested")
        .expect("Failed to commit nested repository");

    let helper_marker = root.path().join("credential-helper-ran");
    let helper = format!("!touch {}", helper_marker.display());
    let remote = "https://secret-token@github.com/goobits/nested.git?access_token=hidden";
    run_git_ok(&nested, &["remote", "add", "origin", remote]);
    run_git_ok(&nested, &["config", "credential.helper", &helper]);

    let git_config = IsolatedGitConfig::new("[repos]\n\ttransportPolicy = ssh-only\n")
        .expect("Failed to isolate Git config");
    let mut command = Command::new(env!("CARGO_BIN_EXE_repos"));
    git_config.apply(&mut command);
    let output = command
        .args(["nested", "update", "shared"])
        .current_dir(&parent)
        .output()
        .expect("Failed to run nested update");
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(!output.status.success(), "HTTPS fetch must be blocked");
    assert!(
        rendered.contains("ssh-only policy blocked fetch: remote origin uses HTTPS"),
        "{rendered}"
    );
    assert!(!rendered.contains("secret-token"), "{rendered}");
    assert!(!rendered.contains("access_token"), "{rendered}");
    assert!(!rendered.contains("hidden"), "{rendered}");
    assert!(
        !helper_marker.exists(),
        "credential helper must not run for blocked nested fetch"
    );
}
