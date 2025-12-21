//! Cargo package publishing functionality

use serde::Deserialize;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

use super::PackageInfo;

const CARGO_OPERATION_TIMEOUT_SECS: u64 = 600; // 10 minutes for cargo operations (can be slow)

/// Cargo.toml package section (partial)
#[derive(Deserialize)]
struct CargoToml {
    package: CargoPackage,
}

#[derive(Deserialize)]
struct CargoPackage {
    name: String,
    version: String,
}

/// Gets package information from Cargo.toml
pub async fn get_package_info(repo_path: &Path) -> Option<PackageInfo> {
    let cargo_toml_path = repo_path.join("Cargo.toml");

    let content = tokio::fs::read_to_string(&cargo_toml_path).await.ok()?;
    let cargo: CargoToml = toml::from_str(&content).ok()?;

    Some(PackageInfo {
        manager: super::PackageManager::Cargo,
        name: cargo.package.name,
        version: cargo.package.version,
    })
}

/// Publishes a cargo package
/// Returns (success, message)
pub async fn publish(repo_path: &Path, dry_run: bool) -> (bool, String) {
    let mut args = vec!["publish"];

    if dry_run {
        args.push("--dry-run");
    }

    let timeout_duration = Duration::from_secs(CARGO_OPERATION_TIMEOUT_SECS);

    let result = tokio::time::timeout(
        timeout_duration,
        Command::new("cargo")
            .args(&args)
            .current_dir(repo_path)
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            if output.status.success() {
                if dry_run {
                    (true, "dry-run ok".to_string())
                } else {
                    (true, "published".to_string())
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let error_message = clean_cargo_error(&stderr);

                // Check if it's an "already published" error
                if stderr.contains("already uploaded")
                    || stderr.contains("crate version") && stderr.contains("already exists")
                {
                    (true, "already published".to_string())
                } else {
                    (false, error_message)
                }
            }
        }
        Ok(Err(e)) => (false, format!("cargo command failed: {e}")),
        Err(_) => (false, "cargo operation timed out".to_string()),
    }
}

/// Cleans up cargo error messages to be more user-friendly
fn clean_cargo_error(error: &str) -> String {
    if error.contains("already uploaded") || error.contains("already exists") {
        "already published".to_string()
    } else if error.contains("no token found") || error.contains("not logged in") {
        "not authenticated (run: cargo login)".to_string()
    } else if error.contains("forbidden") || error.contains("403") {
        "permission denied (check crates.io permissions)".to_string()
    } else if error.contains("Caused by:") {
        // Extract the actual cause
        error
            .lines()
            .skip_while(|line| !line.contains("Caused by:"))
            .nth(1).map_or_else(|| error.trim().to_string(), |line| line.trim().to_string())
    } else {
        // Return first meaningful line
        error
            .lines()
            .find(|line| {
                !line.trim().is_empty()
                    && !line.contains("Uploading")
                    && !line.contains("Packaging")
            }).map_or_else(|| error.trim().to_string(), |line| line.trim().to_string())
    }
}
