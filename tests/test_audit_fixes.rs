use anyhow::Result;
use goobits_repos::audit::fixes::{apply_fixes, FixOptions};
use goobits_repos::audit::hygiene::{check_repo_hygiene, HygieneStatistics};
use goobits_repos::audit::scanner::TruffleStatistics;
use std::process::Command;
use tempfile::TempDir;

mod common;
use common::git::{add_bare_remote, create_test_commit, setup_git_repo};

#[tokio::test]
async fn test_fix_gitignore_violations() -> Result<()> {
    // 1. Setup
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    setup_git_repo(repo_path)?;

    // Create a violation file (.log files are in UNIVERSAL_BAD_PATTERNS)
    create_test_commit(repo_path, "app.log", "some logs", "Add log file")?;

    // 2. Scan
    let (status, message, violations) = check_repo_hygiene(repo_path).await;

    // We expect violations
    assert!(violations.iter().any(|v| v.file_path == "app.log"));

    // 3. Construct Stats
    let mut stats = HygieneStatistics::new();
    stats.update(
        "test-repo",
        repo_path.to_str().unwrap(),
        &status,
        &message,
        violations,
    );

    // 4. Apply Fixes
    let options = FixOptions {
        interactive: false,
        fix_gitignore: true,
        fix_large: false,
        fix_secrets: false,
        untrack_files: true,
        dry_run: false,
        skip_confirm: true,
    };

    let repositories = vec![("test-repo".to_string(), repo_path.to_path_buf())];
    let results = apply_fixes(&repositories, &TruffleStatistics::new(), &stats, options).await?;

    // 5. Verify
    assert_eq!(results.len(), 1);
    assert!(results[0].errors.is_empty());

    // Check .gitignore
    let gitignore_content = std::fs::read_to_string(repo_path.join(".gitignore"))?;
    assert!(gitignore_content.contains("*.log"));

    // Check if file is untracked
    let status_output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_path)
        .output()?;
    let status_text = String::from_utf8(status_output.stdout)?;

    // ?? app.log means it's untracked
    // But since it is now in .gitignore, it should NOT show up in status at all!
    // And .gitignore is committed, so it shouldn't show up either.
    // So status should be empty.
    assert!(
        status_text.trim().is_empty(),
        "Status should be empty (files ignored), got: {}",
        status_text
    );

    Ok(())
}

#[tokio::test]
async fn test_fix_large_files_dry_run() -> Result<()> {
    // 1. Setup
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    setup_git_repo(repo_path)?;

    // Create a large file
    // 1MB + 1 byte
    let size = 1_048_577;
    let large_data = vec![0u8; size];
    std::fs::write(repo_path.join("large.bin"), &large_data)?;

    Command::new("git")
        .args(["add", "large.bin"])
        .current_dir(repo_path)
        .output()?;

    Command::new("git")
        .args(["commit", "-m", "Add large file"])
        .current_dir(repo_path)
        .output()?;

    // 2. Scan
    let (status, message, violations) = check_repo_hygiene(repo_path).await;

    // Verify we found it (requires git cat-file to work in scanner)
    assert!(!violations.is_empty());

    // 3. Construct Stats
    let mut stats = HygieneStatistics::new();
    stats.update(
        "test-repo",
        repo_path.to_str().unwrap(),
        &status,
        &message,
        violations,
    );

    // 4. Apply Fixes (Dry Run)
    let options = FixOptions {
        interactive: false,
        fix_gitignore: false,
        fix_large: true,
        fix_secrets: false,
        untrack_files: false,
        dry_run: true,
        skip_confirm: true,
    };

    let repositories = vec![("test-repo".to_string(), repo_path.to_path_buf())];
    let results = apply_fixes(&repositories, &TruffleStatistics::new(), &stats, options).await?;

    // 5. Verify
    assert_eq!(results.len(), 1);
    assert!(!results[0].fixes_applied.is_empty());
    assert!(results[0].fixes_applied[0].contains("[DRY RUN]"));

    Ok(())
}

#[tokio::test]
async fn test_hygiene_scan_fails_for_non_repository() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let (status, _, violations) = check_repo_hygiene(temp_dir.path()).await;

    assert!(matches!(
        status,
        goobits_repos::audit::hygiene::report::HygieneStatus::Error
    ));
    assert!(violations.is_empty());
    Ok(())
}

#[tokio::test]
async fn test_history_rewrite_fails_when_upstream_is_unreachable() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    setup_git_repo(repo_path)?;

    let large_file = "large.bin";
    std::fs::write(repo_path.join(large_file), vec![0u8; 1_048_577])?;
    let commit = Command::new("git")
        .args(["add", large_file])
        .current_dir(repo_path)
        .output()?;
    assert!(commit.status.success());
    let commit = Command::new("git")
        .args(["commit", "-m", "Add large file"])
        .current_dir(repo_path)
        .output()?;
    assert!(commit.status.success());

    let (status, message, violations) = check_repo_hygiene(repo_path).await;
    let mut stats = HygieneStatistics::new();
    stats.update(
        "test-repo",
        repo_path.to_str().unwrap(),
        &status,
        &message,
        violations,
    );

    let remote = add_bare_remote(repo_path, true)?;
    remote.close()?;

    let repositories = vec![("test-repo".to_string(), repo_path.to_path_buf())];
    let result = apply_fixes(
        &repositories,
        &TruffleStatistics::new(),
        &stats,
        FixOptions {
            interactive: false,
            fix_gitignore: false,
            fix_large: true,
            fix_secrets: false,
            untrack_files: false,
            dry_run: false,
            skip_confirm: true,
        },
    )
    .await;

    let error = result.expect_err("unreachable upstream must block history rewriting");
    assert!(error.to_string().contains("git fetch failed"));
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn test_history_rewrite_blocks_disallowed_fetch_before_credentials() -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Stdio;

    let temp_dir = TempDir::new()?;
    let scan_root = temp_dir.path().join("scan-root");
    let repo_path = scan_root.join("blocked-repo");
    let mock_bin = temp_dir.path().join("mock-bin");
    std::fs::create_dir_all(&repo_path)?;
    std::fs::create_dir_all(&mock_bin)?;
    setup_git_repo(&repo_path)?;
    std::fs::write(repo_path.join("large.bin"), vec![0u8; 1_048_577])?;
    for args in [
        vec!["add", "large.bin"],
        vec!["commit", "-m", "Add large file"],
    ] {
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
    let _remote = add_bare_remote(&repo_path, true)?;
    let remote_url = "https://github.com/goobits/blocked-history-rewrite.git";
    let helper_marker = repo_path.join("credential-helper-ran");
    let helper = format!("!touch {}", helper_marker.display());
    for args in [
        vec!["remote", "set-url", "origin", remote_url],
        vec!["config", "credential.helper", &helper],
    ] {
        let output = Command::new("git")
            .args(args)
            .current_dir(&repo_path)
            .output()?;
        assert!(output.status.success());
    }

    let trufflehog = mock_bin.join("trufflehog");
    std::fs::write(
        &trufflehog,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\nexit 0\n",
    )?;
    let mut permissions = std::fs::metadata(&trufflehog)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&trufflehog, permissions)?;

    let mut paths = vec![mock_bin];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let mut child = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args(["audit", "--fix-large", "--repos", "blocked-repo"])
        .current_dir(&scan_root)
        .env("PATH", std::env::join_paths(paths)?)
        .env("REPOS_TRANSPORT_POLICY", "ssh-only")
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
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "history rewrite should be blocked"
    );
    assert!(
        stderr.contains("ssh-only policy blocked fetch: remote origin uses HTTPS"),
        "{stderr}"
    );
    assert!(
        !helper_marker.exists(),
        "blocked fetch must not invoke a credential helper"
    );
    Ok(())
}

#[tokio::test]
async fn test_existing_gitignore_pattern_still_untracks_tracked_file() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    setup_git_repo(repo_path)?;
    create_test_commit(repo_path, "app.log", "logs", "Add tracked log")?;
    create_test_commit(repo_path, ".gitignore", "*.log\n", "Ignore logs")?;

    let (status, message, violations) = check_repo_hygiene(repo_path).await;
    assert!(violations
        .iter()
        .any(|violation| violation.file_path == "app.log"));
    let mut stats = HygieneStatistics::new();
    stats.update(
        "test-repo",
        repo_path.to_str().expect("UTF-8 path"),
        &status,
        &message,
        violations,
    );
    let repositories = vec![("test-repo".to_string(), repo_path.to_path_buf())];
    let results = apply_fixes(
        &repositories,
        &TruffleStatistics::new(),
        &stats,
        FixOptions {
            interactive: false,
            fix_gitignore: true,
            fix_large: false,
            fix_secrets: false,
            untrack_files: true,
            dry_run: false,
            skip_confirm: true,
        },
    )
    .await?;

    assert!(results[0].errors.is_empty(), "{:?}", results[0].errors);
    let tracked = Command::new("git")
        .args(["ls-files", "--", "app.log"])
        .current_dir(repo_path)
        .output()?;
    assert!(tracked.status.success());
    assert!(tracked.stdout.is_empty(), "app.log should be untracked");
    assert!(
        repo_path.join("app.log").exists(),
        "working file must remain"
    );
    Ok(())
}

#[tokio::test]
async fn test_large_file_scan_retains_more_than_display_limit() -> Result<()> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path();
    setup_git_repo(repo_path)?;
    for index in 0..11u8 {
        let mut contents = vec![0u8; 1_048_577];
        contents[0] = index;
        std::fs::write(repo_path.join(format!("large-{index}.bin")), contents)?;
    }
    let add = Command::new("git")
        .args(["add", "--", "."])
        .current_dir(repo_path)
        .output()?;
    assert!(add.status.success());
    let commit = Command::new("git")
        .args(["commit", "-m", "Add large files"])
        .current_dir(repo_path)
        .output()?;
    assert!(commit.status.success());

    let (_, _, violations) = check_repo_hygiene(repo_path).await;
    let large_count = violations
        .iter()
        .filter(|violation| {
            matches!(
                violation.violation_type,
                goobits_repos::audit::hygiene::ViolationType::LargeFile
            )
        })
        .count();
    assert_eq!(large_count, 11);
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn test_secret_only_repository_is_included_in_dry_run_fixes() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    use std::process::Stdio;

    let temp_dir = TempDir::new()?;
    let scan_root = temp_dir.path().join("scan-root");
    let repo_path = scan_root.join("secret-repo");
    let mock_bin = temp_dir.path().join("mock-bin");
    std::fs::create_dir_all(&repo_path)?;
    std::fs::create_dir_all(&mock_bin)?;
    setup_git_repo(&repo_path)?;
    create_test_commit(&repo_path, "README.md", "clean", "Initial")?;

    let trufflehog = mock_bin.join("trufflehog");
    std::fs::write(
        &trufflehog,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\nprintf '%s\\n' '{\"DetectorName\":\"Test\",\"Verified\":false,\"Raw\":\"safe-token\",\"SourceMetadata\":{\"Data\":{\"Git\":{\"file\":\"README.md\"}}}}'\n",
    )?;
    let mut permissions = std::fs::metadata(&trufflehog)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&trufflehog, permissions)?;

    let mut paths = vec![mock_bin];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_repos"))
        .args([
            "audit",
            "--json",
            "--fix-secrets",
            "--dry-run",
            "--repos",
            "secret-repo",
        ])
        .current_dir(&scan_root)
        .env("PATH", std::env::join_paths(paths)?)
        .stdin(Stdio::null())
        .output()?;
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(json["fixes"]["successful_repos"], 1);
    assert!(json["fixes"]["results"][0]["fixes_applied"][0]
        .as_str()
        .is_some_and(|message| message.contains("DRY RUN")));
    Ok(())
}
