use anyhow::Result;
use goobits_repos::audit::fixes::{apply_fixes, FixOptions};
use goobits_repos::audit::hygiene::{check_repo_hygiene, HygieneStatistics};
use goobits_repos::audit::scanner::TruffleStatistics;
use std::process::Command;
use tempfile::TempDir;

mod common;
use common::git::{create_test_commit, setup_git_repo};

fn history_rewrite_tools_available() -> bool {
    let filter_repo = Command::new("git")
        .args(["filter-repo", "--version"])
        .output()
        .is_ok_and(|output| output.status.success());
    let git_is_compatible = Command::new("git")
        .args(["cat-file", "-h"])
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout).contains("--batch-command")
                || String::from_utf8_lossy(&output.stderr).contains("--batch-command")
        });
    if !filter_repo || !git_is_compatible {
        eprintln!("Git 2.36+ and git-filter-repo are unavailable; skipping destructive test");
    }
    filter_repo && git_is_compatible
}

#[tokio::test]
async fn test_fix_large_actual_removal() -> Result<()> {
    if !history_rewrite_tools_available() {
        return Ok(());
    }

    // 1. Setup repo with a large file in history
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    setup_git_repo(repo_path)?;

    let large_file = "huge.dat";
    let size = 1_048_577; // > 1MB
    std::fs::write(repo_path.join(large_file), vec![0u8; size])?;

    Command::new("git")
        .args(["add", large_file])
        .current_dir(repo_path)
        .output()?;
    Command::new("git")
        .args(["commit", "-m", "Add large file"])
        .current_dir(repo_path)
        .output()?;

    // 2. Scan
    let (status, message, violations) = check_repo_hygiene(repo_path).await;
    assert!(!violations.is_empty());

    let mut stats = HygieneStatistics::new();
    stats.update(
        "test-repo",
        repo_path.to_str().unwrap(),
        &status,
        &message,
        violations,
    );

    // 3. Fix
    let options = FixOptions {
        interactive: false,
        fix_gitignore: false,
        fix_large: true,
        fix_secrets: false,
        untrack_files: false,
        dry_run: false,
        skip_confirm: true,
    };

    let repositories = vec![("test-repo".to_string(), repo_path.to_path_buf())];
    apply_fixes(&repositories, &TruffleStatistics::new(), &stats, options).await?;

    // 4. Verify gone from history
    let output = Command::new("git")
        .args(["rev-list", "--objects", "--all"])
        .current_dir(repo_path)
        .output()?;
    let history = String::from_utf8_lossy(&output.stdout);
    assert!(
        !history.contains(large_file),
        "File should be removed from history"
    );

    Ok(())
}

#[tokio::test]
async fn test_recovery_from_backup_bundle() -> Result<()> {
    if !history_rewrite_tools_available() {
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    setup_git_repo(repo_path)?;

    let large_file = "huge.dat";
    std::fs::write(repo_path.join(large_file), vec![0u8; 1_048_577])?;
    Command::new("git")
        .args(["add", large_file])
        .current_dir(repo_path)
        .output()?;
    Command::new("git")
        .args(["commit", "-m", "Add large file"])
        .current_dir(repo_path)
        .output()?;
    let original_head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()?;
    let original_head_hash = String::from_utf8(original_head.stdout)?.trim().to_string();

    let (status, message, violations) = check_repo_hygiene(repo_path).await;
    let mut stats = HygieneStatistics::new();
    stats.update(
        "test-repo",
        repo_path.to_str().unwrap(),
        &status,
        &message,
        violations,
    );

    let options = FixOptions {
        interactive: false,
        fix_gitignore: false,
        fix_large: true,
        fix_secrets: false,
        untrack_files: false,
        dry_run: false,
        skip_confirm: true,
    };

    let repositories = vec![("test-repo".to_string(), repo_path.to_path_buf())];
    let results = apply_fixes(&repositories, &TruffleStatistics::new(), &stats, options).await?;

    // Extract the durable full-ref bundle from the result.
    let fix_msg = &results[0].fixes_applied[0];
    let backup_bundle = fix_msg
        .split("Recovery bundle: ")
        .nth(1)
        .expect("fix result should report a recovery bundle")
        .trim();

    let recovery_root = TempDir::new()?;
    let recovered = recovery_root.path().join("recovered");
    let clone = Command::new("git")
        .arg("clone")
        .arg(backup_bundle)
        .arg(&recovered)
        .output()?;
    assert!(
        clone.status.success(),
        "{}",
        String::from_utf8_lossy(&clone.stderr)
    );

    let new_head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&recovered)
        .output()?;
    let new_head_hash = String::from_utf8(new_head.stdout)?.trim().to_string();

    assert_eq!(
        original_head_hash, new_head_hash,
        "Should have rolled back to original HEAD"
    );
    assert!(
        recovered.join(large_file).exists(),
        "Large file should be present in the recovered clone"
    );

    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn test_secret_rewrite_preserves_same_text_in_unrelated_file() -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Stdio;

    if !history_rewrite_tools_available() {
        return Ok(());
    }

    let temp_dir = TempDir::new()?;
    let scan_root = temp_dir.path().join("scan-root");
    let repo_path = scan_root.join("secret-repo");
    let mock_bin = temp_dir.path().join("mock-bin");
    std::fs::create_dir_all(&repo_path)?;
    std::fs::create_dir_all(&mock_bin)?;
    setup_git_repo(&repo_path)?;
    std::fs::write(repo_path.join("secret.env"), "token=shared-value\n")?;
    std::fs::write(
        repo_path.join("unrelated.txt"),
        "shared-value is ordinary text here\n",
    )?;
    for args in [vec!["add", "--", "."], vec!["commit", "-m", "Add fixture"]] {
        let output = Command::new("git")
            .args(args)
            .current_dir(&repo_path)
            .output()?;
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let trufflehog = mock_bin.join("trufflehog");
    std::fs::write(
        &trufflehog,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\nif git show HEAD:secret.env 2>/dev/null | grep -q 'shared-value'; then\n  printf '%s\\n' '{\"DetectorName\":\"Test\",\"Verified\":false,\"Raw\":\"shared-value\",\"SecretParts\":{\"host\":\"ordinary\"},\"SourceMetadata\":{\"Data\":{\"Git\":{\"file\":\"secret.env\"}}}}'\nfi\n",
    )?;
    let mut permissions = std::fs::metadata(&trufflehog)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&trufflehog, permissions)?;

    let mut paths = vec![mock_bin];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let mut child = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["audit", "--fix-secrets", "--repos", "secret-repo"])
        .current_dir(&scan_root)
        .env("PATH", std::env::join_paths(paths)?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .as_mut()
        .expect("confirmation stdin")
        .write_all(b"yes\n")?;
    let output = child.wait_with_output()?;
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let unrelated = Command::new("git")
        .args(["show", "HEAD:unrelated.txt"])
        .current_dir(&repo_path)
        .output()?;
    assert!(unrelated.status.success());
    assert_eq!(
        String::from_utf8(unrelated.stdout)?,
        "shared-value is ordinary text here\n"
    );
    let redacted = Command::new("git")
        .args(["show", "HEAD:secret.env"])
        .current_dir(&repo_path)
        .output()?;
    assert!(redacted.status.success());
    assert_eq!(String::from_utf8(redacted.stdout)?, "token=REDACTED\n");

    Ok(())
}

#[tokio::test]
async fn test_fix_concurrent_operations() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();

    let mut stats = HygieneStatistics::new();
    let mut repositories = Vec::new();

    for i in 1..=3 {
        let repo_name = format!("repo-{}", i);
        let repo_path = root.join(&repo_name);
        std::fs::create_dir(&repo_path)?;
        setup_git_repo(&repo_path)?;

        create_test_commit(&repo_path, &format!("app-{}.log", i), "logs", "Add log")?;

        let (status, message, violations) = check_repo_hygiene(&repo_path).await;
        stats.update(
            &repo_name,
            repo_path.to_str().unwrap(),
            &status,
            &message,
            violations,
        );
        repositories.push((repo_name, repo_path));
    }

    let options = FixOptions {
        interactive: false,
        fix_gitignore: true,
        fix_large: false,
        fix_secrets: false,
        untrack_files: true,
        dry_run: false,
        skip_confirm: true,
    };

    let results = apply_fixes(&repositories, &TruffleStatistics::new(), &stats, options).await?;
    assert_eq!(results.len(), 3);
    for r in results {
        assert!(r.errors.is_empty());
    }

    Ok(())
}
