//! Cargo package publishing functionality

use async_trait::async_trait;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

use super::{run_package_command, CommandEffect, PackageInfo, PackageManager, PackageManifest};

const CARGO_OPERATION_TIMEOUT_SECS: u64 = 600; // 10 minutes for cargo operations (can be slow)

pub struct Cargo;

#[async_trait]
impl PackageManager for Cargo {
    fn name(&self) -> &str {
        "cargo"
    }

    fn icon(&self) -> &str {
        "📦"
    }

    async fn detect(&self, path: &Path) -> bool {
        tokio::fs::metadata(path.join("Cargo.toml")).await.is_ok()
    }

    async fn get_info(&self, path: &Path) -> Option<PackageInfo> {
        get_package_info_internal(path).await
    }

    async fn dependencies(&self, path: &Path) -> Vec<String> {
        get_manifest(path)
            .await
            .ok()
            .flatten()
            .map(|manifest| {
                manifest
                    .dependencies
                    .into_iter()
                    .chain(manifest.build_dependencies)
                    .map(|(name, value)| renamed_dependency(name, &value))
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn inspect_manifest(&self, path: &Path) -> anyhow::Result<Option<PackageManifest>> {
        let Some(manifest) = get_manifest(path).await? else {
            return Ok(None);
        };
        let dependencies = manifest
            .dependencies
            .into_iter()
            .chain(manifest.build_dependencies)
            .map(|(name, value)| renamed_dependency(name, &value))
            .collect();
        Ok(Some(PackageManifest {
            info: PackageInfo {
                manager_name: "cargo".to_string(),
                name: manifest.package.name,
                version: manifest.package.version,
            },
            dependencies,
        }))
    }

    async fn publish(&self, path: &Path, dry_run: bool) -> (bool, String) {
        publish_internal(path, dry_run).await
    }
}

/// Cargo.toml package section (partial)
#[derive(Deserialize)]
struct CargoToml {
    package: CargoPackage,
    #[serde(default)]
    dependencies: HashMap<String, toml::Value>,
    #[serde(default, rename = "build-dependencies")]
    build_dependencies: HashMap<String, toml::Value>,
}

#[derive(Deserialize)]
struct CargoPackage {
    name: String,
    version: String,
}

fn renamed_dependency(name: String, value: &toml::Value) -> String {
    value
        .as_table()
        .and_then(|table| table.get("package"))
        .and_then(toml::Value::as_str)
        .map_or(name, str::to_string)
}

async fn get_manifest(repo_path: &Path) -> anyhow::Result<Option<CargoToml>> {
    let path = repo_path.join("Cargo.toml");
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(toml::from_str(&content)?))
}

/// Gets package information from Cargo.toml
async fn get_package_info_internal(repo_path: &Path) -> Option<PackageInfo> {
    let cargo = get_manifest(repo_path).await.ok().flatten()?;

    Some(PackageInfo {
        manager_name: "cargo".to_string(),
        name: cargo.package.name,
        version: cargo.package.version,
    })
}

/// Publishes a cargo package
/// Returns (success, message)
async fn publish_internal(repo_path: &Path, dry_run: bool) -> (bool, String) {
    let mut args = vec!["publish"];

    if dry_run {
        args.push("--dry-run");
    }

    let mut command = Command::new("cargo");
    command.args(&args).current_dir(repo_path);
    let effect = if dry_run {
        CommandEffect::Local
    } else {
        CommandEffect::RegistryMutation
    };
    let result = run_package_command(
        command,
        Duration::from_secs(CARGO_OPERATION_TIMEOUT_SECS),
        "cargo publish",
        effect,
    )
    .await;

    match result {
        Ok(output) => {
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
        Err(error) => (false, error.to_string()),
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
            .nth(1)
            .map_or_else(|| error.trim().to_string(), |line| line.trim().to_string())
    } else {
        // Return first meaningful line
        error
            .lines()
            .find(|line| {
                !line.trim().is_empty()
                    && !line.contains("Uploading")
                    && !line.contains("Packaging")
            })
            .map_or_else(|| error.trim().to_string(), |line| line.trim().to_string())
    }
}
