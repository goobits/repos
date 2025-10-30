//! Package management and publishing functionality

pub mod cargo;
pub mod npm;
pub mod pypi;

use anyhow::Result;
use std::path::Path;

/// Supported package managers
#[derive(Clone, Debug, PartialEq)]
pub enum PackageManager {
    Npm,
    Cargo,
    PyPI,
}

impl PackageManager {
    /// Returns the display name for this package manager
    pub fn name(&self) -> &str {
        match self {
            PackageManager::Npm => "npm",
            PackageManager::Cargo => "cargo",
            PackageManager::PyPI => "python",
        }
    }

    /// Returns the emoji icon for this package manager
    pub fn icon(&self) -> &str {
        match self {
            PackageManager::Npm => "📦",
            PackageManager::Cargo => "📦",
            PackageManager::PyPI => "📦",
        }
    }
}

/// Information about a detected package
#[derive(Clone, Debug)]
pub struct PackageInfo {
    pub manager: PackageManager,
    pub name: String,
    pub version: String,
}

/// Detects the package manager for a given repository path
/// Returns None if no package manager is detected
pub fn detect_package_manager(repo_path: &Path) -> Option<PackageManager> {
    if repo_path.join("package.json").exists() {
        Some(PackageManager::Npm)
    } else if repo_path.join("Cargo.toml").exists() {
        Some(PackageManager::Cargo)
    } else if repo_path.join("pyproject.toml").exists() {
        Some(PackageManager::PyPI)
    } else if repo_path.join("setup.py").exists() {
        Some(PackageManager::PyPI)
    } else {
        None
    }
}

/// Gets package information from a repository
pub async fn get_package_info(repo_path: &Path) -> Option<PackageInfo> {
    let manager = detect_package_manager(repo_path)?;

    match manager {
        PackageManager::Npm => npm::get_package_info(repo_path).await,
        PackageManager::Cargo => cargo::get_package_info(repo_path).await,
        PackageManager::PyPI => pypi::get_package_info(repo_path).await,
    }
}

/// Publishes a package using the appropriate package manager
/// Returns (success, message)
pub async fn publish_package(
    repo_path: &Path,
    manager: &PackageManager,
    dry_run: bool,
) -> (bool, String) {
    match manager {
        PackageManager::Npm => npm::publish(repo_path, dry_run).await,
        PackageManager::Cargo => cargo::publish(repo_path, dry_run).await,
        PackageManager::PyPI => pypi::publish(repo_path, dry_run).await,
    }
}

/// Status of a publish operation
#[derive(Clone, Debug)]
pub enum PublishStatus {
    /// Package was successfully published
    Published,
    /// Package is already published (version exists)
    AlreadyPublished,
    /// Package was skipped (no package manager detected)
    Skipped,
    /// An error occurred during publishing
    Error,
    /// Dry run completed successfully
    DryRunOk,
}

impl PublishStatus {
    /// Returns the emoji symbol for this status
    pub fn symbol(&self) -> &str {
        match self {
            PublishStatus::Published | PublishStatus::DryRunOk => "🟢",
            PublishStatus::AlreadyPublished | PublishStatus::Skipped => "🟠",
            PublishStatus::Error => "🔴",
        }
    }

    /// Returns the text representation of this status
    pub fn text(&self) -> &str {
        match self {
            PublishStatus::Published => "published",
            PublishStatus::AlreadyPublished => "already-published",
            PublishStatus::Skipped => "skipped",
            PublishStatus::Error => "failed",
            PublishStatus::DryRunOk => "ok",
        }
    }
}
