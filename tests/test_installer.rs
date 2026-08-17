#![cfg(unix)]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

fn run_installer(home: &Path, install_dir: &Path, cache_home: &Path) -> Output {
    Command::new("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("HOME", home)
        .env("SHELL", "/bin/zsh")
        .env("REPOS_INSTALL_DIR", install_dir)
        .env("XDG_CACHE_HOME", cache_home)
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("REPOS_SKIP_PATH_SETUP")
        .output()
        .expect("installer should start")
}

fn assert_success(output: &Output, operation: &str) {
    assert!(
        output.status.success(),
        "{operation} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_no_installer_artifacts(install_dir: &Path) {
    let artifacts = fs::read_dir(install_dir)
        .expect("install directory should be readable")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| {
            let name = name.to_string_lossy();
            name.starts_with(".repos.install.") || name.starts_with(".repos.backup.")
        })
        .collect::<Vec<_>>();
    assert!(artifacts.is_empty(), "stale installer files: {artifacts:?}");
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("mock executable should be written");
    let mut permissions = fs::metadata(path)
        .expect("mock executable should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("mock executable should be executable");
}

#[test]
fn installer_rejects_toolchains_that_cannot_read_the_lockfile() {
    let sandbox = TempDir::new().expect("installer sandbox should be created");
    let mock_bin = sandbox.path().join("mock-bin");
    let home = sandbox.path().join("home");
    fs::create_dir_all(&mock_bin).expect("mock bin should be created");
    fs::create_dir_all(&home).expect("test home should be created");
    write_executable(
        &mock_bin.join("cargo"),
        "#!/bin/sh\nprintf '%s\\n' 'cargo 1.77.0 (mock)'\n",
    );
    write_executable(
        &mock_bin.join("rustc"),
        "#!/bin/sh\nprintf '%s\\n' 'rustc 1.77.0 (mock)'\n",
    );

    let output = Command::new("bash")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("install.sh"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("HOME", &home)
        .env("PATH", format!("{}:/usr/bin:/bin", mock_bin.display()))
        .env("SHELL", "/bin/zsh")
        .output()
        .expect("installer should start");

    assert!(!output.status.success(), "old toolchain should be rejected");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("requires Cargo and Rust 1.78 or newer"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn installer_atomically_updates_with_safe_permissions_and_shell_paths() {
    if !Command::new("bash")
        .arg("-c")
        .arg("command -v cargo >/dev/null && command -v rustc >/dev/null")
        .status()
        .is_ok_and(|status| status.success())
    {
        eprintln!("skipping installer test because the Rust toolchain is unavailable");
        return;
    }

    let sandbox = TempDir::new().expect("installer sandbox should be created");
    let home = sandbox.path().join("home");
    let install_dir = sandbox.path().join("install & user's tools").join("bin");
    let cache_home = sandbox.path().join("cache");
    fs::create_dir_all(&home).expect("test home should be created");

    let first = run_installer(&home, &install_dir, &cache_home);
    assert_success(&first, "initial installation");

    let canonical_install_dir =
        fs::canonicalize(&install_dir).expect("install directory should canonicalize");
    let installed_binary = canonical_install_dir.join("repos");
    let first_metadata = fs::metadata(&installed_binary).expect("repos should be installed");
    assert_eq!(first_metadata.permissions().mode() & 0o777, 0o755);
    let first_inode = first_metadata.ino();

    let cache_metadata = fs::metadata(cache_home.join("goobits-repos"))
        .expect("private installer cache should exist");
    assert_eq!(cache_metadata.permissions().mode() & 0o777, 0o700);

    let shell_config =
        fs::read_to_string(home.join(".zshrc")).expect("zsh configuration should be created");
    assert!(shell_config.contains(". \"$HOME/.repos-env\""));

    let configured_path = Command::new("/bin/sh")
        .arg("-c")
        .arg(". \"$HOME/.repos-env\"; printf '%s' \"$PATH\"")
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("generated environment should be sourceable");
    assert_success(&configured_path, "sourcing generated PATH configuration");
    assert_eq!(
        String::from_utf8(configured_path.stdout).expect("PATH should be UTF-8"),
        format!("{}:/usr/bin:/bin", canonical_install_dir.display())
    );

    assert_success(
        &Command::new(&installed_binary)
            .arg("--version")
            .output()
            .expect("installed binary should run"),
        "running initial installation",
    );

    let second = run_installer(&home, &install_dir, &cache_home);
    assert_success(&second, "updating existing installation");
    let second_metadata = fs::metadata(&installed_binary).expect("repos should remain installed");
    assert_eq!(second_metadata.permissions().mode() & 0o777, 0o755);
    assert_ne!(
        first_inode,
        second_metadata.ino(),
        "updates must replace rather than overwrite the executable inode"
    );
    assert_no_installer_artifacts(&install_dir);
}
