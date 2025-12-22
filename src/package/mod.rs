//! Package management and publishing functionality
//!
//! This module provides an extensible system for detecting and publishing packages
//! across different ecosystems (npm, Cargo, PyPI).

pub mod cargo;
pub mod npm;
pub mod pypi;

use std::path::Path;
use std::sync::Arc;
use async_trait::async_trait;

/// Trait for package managers to implement.
///
/// Implementors can define how to detect their package type, how to extract
/// version information, and how to perform the actual publishing operation.
#[async_trait]
pub trait PackageManager: Send + Sync {
    /// Returns the display name for this package manager (e.g., "npm", "cargo").
    fn name(&self) -> &str;
    
    /// Returns the emoji icon for this package manager.
    fn icon(&self) -> &str;
    
    /// Detects if this package manager manages the repository at the given path.
    async fn detect(&self, path: &Path) -> bool;
    
    /// Gets package information (name, version) from the repository.
    async fn get_info(&self, path: &Path) -> Option<PackageInfo>;
    
    /// Publishes the package to its respective registry.
    ///
    /// Returns `(success, message)`.
    async fn publish(&self, path: &Path, dry_run: bool) -> (bool, String);
}

/// Information about a detected package.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct PackageInfo {
    /// The name of the package manager that detected this package.
    pub manager_name: String,
    /// The name of the package as defined in its manifest.
    pub name: String,
    /// The version of the package.
    pub version: String,
}


/// Returns a list of all supported package managers
pub fn get_all_managers() -> Vec<Arc<dyn PackageManager>> {
    vec![
        Arc::new(npm::Npm),
        Arc::new(cargo::Cargo),
        Arc::new(pypi::PyPI),
    ]
}

/// Helper to detect package manager for a path (returns the first match)
pub async fn detect_manager(path: &Path) -> Option<Arc<dyn PackageManager>> {
    // Check in order of priority: Npm, Cargo, PyPI
    // (Npm first because it's common to have package.json alongside others)
    let managers = get_all_managers();
    
    for manager in managers {
        if manager.detect(path).await {
            return Some(manager);
        }
    }
    None
}

/// Status of a publish operation
#[derive(Clone, Debug)]
#[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_detect_npm_package() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        std::fs::write(temp_dir.path().join("package.json"), "{}").expect("Failed to write");
        
        let manager = detect_manager(temp_dir.path()).await;
        assert!(manager.is_some());
        assert_eq!(manager.unwrap().name(), "npm");
    }

    #[tokio::test]
    async fn test_detect_cargo_package() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        std::fs::write(temp_dir.path().join("Cargo.toml"), "").expect("Failed to write");
        
        let manager = detect_manager(temp_dir.path()).await;
        assert!(manager.is_some());
        assert_eq!(manager.unwrap().name(), "cargo");
    }

    #[tokio::test]
    async fn test_detect_pypi_package() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        std::fs::write(temp_dir.path().join("pyproject.toml"), "").expect("Failed to write");
        
        let manager = detect_manager(temp_dir.path()).await;
        assert!(manager.is_some());
        assert_eq!(manager.unwrap().name(), "python");
    }

    #[tokio::test]
    async fn test_detect_none() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        
        let manager = detect_manager(temp_dir.path()).await;
        assert!(manager.is_none());
    }
}
