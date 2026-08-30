//! TruffleHog availability checks and checksum-verified installation.

use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tokio::process::Command;

const TRUFFLEHOG_INSTALL_SCRIPT_SHA256: &str =
    "c394defeaea8a7c48f828a2051b608a9b19f43f34b891407b66a386c3e2591e2";

pub(super) async fn is_trufflehog_installed() -> bool {
    Command::new("trufflehog")
        .arg("--version")
        .output()
        .await
        .map(|output| output.status.success())
        .unwrap_or(false)
}

pub(super) async fn ensure_trufflehog_installed() -> Result<()> {
    if is_trufflehog_installed().await {
        eprintln!("✅ TruffleHog is already installed");
        return Ok(());
    }

    eprintln!("📦 Installing TruffleHog...");

    let install_cmd = if cfg!(target_os = "macos") {
        if Command::new("brew").arg("--version").output().await.is_ok() {
            (
                "brew",
                vec!["install", "trufflesecurity/trufflehog/trufflehog"],
            )
        } else {
            return install_trufflehog_direct().await;
        }
    } else if cfg!(target_os = "linux") {
        return install_trufflehog_direct().await;
    } else {
        return Err(anyhow!("Automatic TruffleHog installation not supported on this platform. Please install manually."));
    };

    eprintln!("Running: {} {}", install_cmd.0, install_cmd.1.join(" "));

    let output = Command::new(install_cmd.0)
        .args(&install_cmd.1)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("Failed to install TruffleHog: {stderr}"));
    }

    if !is_trufflehog_installed().await {
        return Err(anyhow!(
            "TruffleHog installation completed but tool is not accessible"
        ));
    }

    eprintln!("✅ TruffleHog installed successfully");
    Ok(())
}

async fn verify_file_checksum(path: &Path, expected_sha256: &str) -> Result<bool> {
    if expected_sha256 == "PLACEHOLDER_UPDATE_WITH_ACTUAL_CHECKSUM" {
        return Ok(false);
    }

    let contents = tokio::fs::read(path).await?;
    let mut hasher = Sha256::new();
    hasher.update(&contents);
    let result = hasher.finalize();
    let computed_hash = format!("{result:x}");

    Ok(computed_hash.eq_ignore_ascii_case(expected_sha256))
}

async fn install_trufflehog_direct() -> Result<()> {
    eprintln!("\n⚠️  SECURITY NOTICE:");
    eprintln!("   This will download and execute an installation script from:");
    eprintln!(
        "   https://raw.githubusercontent.com/trufflesecurity/trufflehog/main/scripts/install.sh"
    );
    eprintln!("   The script will be verified against a known checksum before execution.\n");

    eprintln!("📥 Downloading TruffleHog installation script...");

    let script_url =
        "https://raw.githubusercontent.com/trufflesecurity/trufflehog/main/scripts/install.sh";
    let home = PathBuf::from(std::env::var("HOME")?);
    let install_path = select_install_path(Path::new("/usr/local/bin"), &home).await?;

    let script_workspace = create_script_workspace()?;
    let temp_script = script_workspace.path().join("install.sh");
    let download_output = Command::new("curl")
        .args(["-sSfL", "-o"])
        .arg(&temp_script)
        .arg(script_url)
        .output()
        .await?;

    if !download_output.status.success() {
        let stderr = String::from_utf8_lossy(&download_output.stderr);
        return Err(anyhow!("Failed to download TruffleHog installer: {stderr}"));
    }

    eprintln!("✅ Download complete, verifying checksum...");

    match verify_file_checksum(&temp_script, TRUFFLEHOG_INSTALL_SCRIPT_SHA256).await {
        Ok(true) => eprintln!("✅ Checksum verification passed"),
        Ok(false) => {
            return Err(anyhow!(
                "TruffleHog installer checksum did not match the trusted value"
            ));
        }
        Err(error) => {
            return Err(anyhow!(
                "TruffleHog installer checksum check failed: {error}"
            ));
        }
    }

    eprintln!("🔧 Executing installation script...");
    let install_output = Command::new("sh")
        .arg(&temp_script)
        .arg("-b")
        .arg(&install_path)
        .output()
        .await?;

    if !install_output.status.success() {
        let stderr = String::from_utf8_lossy(&install_output.stderr);
        return Err(anyhow!("Failed to install TruffleHog: {stderr}"));
    }

    Ok(())
}

async fn select_install_path(system_bin: &Path, home: &Path) -> Result<PathBuf> {
    if tokio::fs::metadata(system_bin).await.is_ok() && directory_is_writable(system_bin) {
        return Ok(system_bin.to_path_buf());
    }

    let user_bin = home.join(".local/bin");
    tokio::fs::create_dir_all(&user_bin).await?;
    eprintln!(
        "⚠️  Installing to {} (add to PATH if needed)",
        user_bin.display()
    );
    Ok(user_bin)
}

fn directory_is_writable(path: &Path) -> bool {
    tempfile::Builder::new()
        .prefix(".repos-write-test-")
        .tempfile_in(path)
        .is_ok()
}

fn create_script_workspace() -> Result<TempDir> {
    let workspace = tempfile::Builder::new()
        .prefix("repos-trufflehog-")
        .tempdir()
        .context("Failed to create a private installer workspace")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::set_permissions(workspace.path(), std::fs::Permissions::from_mode(0o700))
            .context("Failed to secure the installer workspace")?;
    }
    Ok(workspace)
}

#[cfg(test)]
mod tests {
    use super::{create_script_workspace, directory_is_writable, select_install_path};

    #[test]
    fn writable_probe_preserves_existing_files_and_leaves_no_artifact() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let sentinel = directory.path().join("test_write");
        std::fs::write(&sentinel, "keep me").expect("write sentinel");
        let before = std::fs::read_dir(directory.path())
            .expect("read directory")
            .count();

        assert!(directory_is_writable(directory.path()));
        assert_eq!(
            std::fs::read_to_string(sentinel).expect("read sentinel"),
            "keep me"
        );
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("read directory")
                .count(),
            before
        );
    }

    #[tokio::test]
    async fn install_path_falls_back_to_private_user_bin() {
        let root = tempfile::tempdir().expect("temporary directory");
        let unavailable_system_bin = root.path().join("missing/system/bin");
        let home = root.path().join("home");

        let selected = select_install_path(&unavailable_system_bin, &home)
            .await
            .expect("select install path");

        assert_eq!(selected, home.join(".local/bin"));
        assert!(selected.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn installer_workspace_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let workspace = create_script_workspace().expect("create workspace");
        let mode = workspace
            .path()
            .metadata()
            .expect("workspace metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);
    }
}
