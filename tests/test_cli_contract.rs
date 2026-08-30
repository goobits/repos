use std::process::Command;

use tempfile::TempDir;

#[test]
fn save_help_exposes_one_canonical_untracked_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["save", "--help"])
        .output()
        .expect("Failed to run save help");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("-a, --all"), "{stdout}");
    assert!(!stdout.contains("--include-untracked"), "{stdout}");
    assert!(!stdout.contains("-u,"), "{stdout}");
}

#[test]
fn legacy_save_untracked_aliases_remain_accepted() {
    let directory = TempDir::new().expect("Failed to create temporary directory");
    for alias in ["--include-untracked", "-u"] {
        let output = Command::new(env!("CARGO_BIN_EXE_repos"))
            .args(["save", "compatibility check", alias, "--dry-run"])
            .current_dir(directory.path())
            .output()
            .expect("Failed to run save alias");
        assert!(
            output.status.success(),
            "{alias} must remain accepted: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn pull_help_describes_fast_forward_and_rebase_contracts() {
    let output = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["pull", "--help"])
        .output()
        .expect("Failed to run pull help");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("Rebase diverged branches"), "{stdout}");
    assert!(stdout.contains("fast-forward"), "{stdout}");
    assert!(!stdout.contains("instead of merge"), "{stdout}");
}
