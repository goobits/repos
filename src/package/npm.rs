//! NPM package publishing functionality

use serde::Deserialize;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;
use async_trait::async_trait;

use super::{PackageInfo, PackageManager};

const NPM_OPERATION_TIMEOUT_SECS: u64 = 300; // 5 minutes for npm operations

pub struct Npm;

#[async_trait]
impl PackageManager for Npm {
    fn name(&self) -> &str {
        "npm"
    }

    fn icon(&self) -> &str {
        "📦"
    }

    async fn detect(&self, path: &Path) -> bool {
        tokio::fs::metadata(path.join("package.json")).await.is_ok()
    }

    async fn get_info(&self, path: &Path) -> Option<PackageInfo> {
        get_package_info_internal(path).await
    }

    async fn publish(&self, path: &Path, dry_run: bool) -> (bool, String) {
        publish_internal(path, dry_run).await
    }
}

/// npm package.json structure (partial)
#[derive(Deserialize)]
struct PackageJson {
    name: String,
    version: String,
}

/// Gets package information from package.json
async fn get_package_info_internal(repo_path: &Path) -> Option<PackageInfo> {
    let package_json_path = repo_path.join("package.json");

    let content = tokio::fs::read_to_string(&package_json_path).await.ok()?;
    let package: PackageJson = serde_json::from_str(&content).ok()?;

    Some(PackageInfo {
        manager_name: "npm".to_string(),
        name: package.name,
        version: package.version,
    })
}

/// Publishes an npm package
/// Returns (success, message)
async fn publish_internal(repo_path: &Path, dry_run: bool) -> (bool, String) {
    let mut args = vec!["publish"];

    if dry_run {
        args.push("--dry-run");
    }

    let timeout_duration = Duration::from_secs(NPM_OPERATION_TIMEOUT_SECS);

    let result = tokio::time::timeout(
        timeout_duration,
        Command::new("npm")
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
                    // Check if it was actually published or already exists
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if stderr.contains("You cannot publish over the previously published versions")
                        || stderr.contains("cannot publish over existing version")
                    {
                        (true, "already published".to_string())
                    } else {
                        (true, "published".to_string())
                    }
                }
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let error_message = clean_npm_error(&stderr);
                (false, error_message)
            }
        }
        Ok(Err(e)) => (false, format!("npm command failed: {e}")),
        Err(_) => (false, "npm operation timed out".to_string()),
    }
}

/// Cleans up npm error messages to be more user-friendly
fn clean_npm_error(error: &str) -> String {
    // Extract the most relevant error message
    if error.contains("You cannot publish over the previously published versions") {
        "already published".to_string()
    } else if error.contains("You must be logged in") || error.contains("need auth") {
        "not authenticated (run: npm login)".to_string()
    } else if error.contains("403") {
        "permission denied (check npm permissions)".to_string()
    } else if error.contains("404") {
        "registry not found".to_string()
    } else {
        // Return first line of error, cleaned up
        error
            .lines()
            .find(|line| !line.trim().is_empty() && line.contains("npm ERR!")).map_or_else(|| error.trim().to_string(), |line| line.replace("npm ERR!", "").trim().to_string())
    }
}
