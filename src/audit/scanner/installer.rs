//! TruffleHog availability checks and checksum-verified installation.

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
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

async fn verify_file_checksum(path: &std::path::Path, expected_sha256: &str) -> Result<bool> {
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
    let install_dir = "/usr/local/bin";

    let install_path = if tokio::fs::metadata(install_dir).await.is_ok()
        && tokio::fs::File::create(format!("{install_dir}/test_write"))
            .await
            .is_ok()
    {
        let _ = tokio::fs::remove_file(format!("{install_dir}/test_write")).await;
        install_dir.to_string()
    } else {
        let home = std::env::var("HOME")?;
        let user_bin = format!("{home}/.local/bin");
        tokio::fs::create_dir_all(&user_bin).await?;
        eprintln!("⚠️  Installing to {user_bin} (add to PATH if needed)");
        user_bin
    };

    let temp_script = format!("/tmp/trufflehog-install-{}.sh", std::process::id());
    let download_output = Command::new("curl")
        .args(["-sSfL", "-o", &temp_script, script_url])
        .output()
        .await?;

    if !download_output.status.success() {
        let stderr = String::from_utf8_lossy(&download_output.stderr);
        return Err(anyhow!("Failed to download TruffleHog installer: {stderr}"));
    }

    eprintln!("✅ Download complete, verifying checksum...");

    let temp_script_path = std::path::Path::new(&temp_script);
    match verify_file_checksum(temp_script_path, TRUFFLEHOG_INSTALL_SCRIPT_SHA256).await {
        Ok(true) => eprintln!("✅ Checksum verification passed"),
        Ok(false) => {
            let _ = tokio::fs::remove_file(&temp_script).await;
            return Err(anyhow!(
                "TruffleHog installer checksum did not match the trusted value"
            ));
        }
        Err(error) => {
            let _ = tokio::fs::remove_file(&temp_script).await;
            return Err(anyhow!(
                "TruffleHog installer checksum check failed: {error}"
            ));
        }
    }

    eprintln!("🔧 Executing installation script...");
    let install_output = Command::new("sh")
        .args([&temp_script, "-b", &install_path])
        .output()
        .await?;

    let _ = tokio::fs::remove_file(&temp_script).await;

    if !install_output.status.success() {
        let stderr = String::from_utf8_lossy(&install_output.stderr);
        return Err(anyhow!("Failed to install TruffleHog: {stderr}"));
    }

    Ok(())
}
