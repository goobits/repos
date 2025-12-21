//! Package management and publishing functionality

pub mod cargo;
pub mod npm;
pub mod pypi;

use std::path::Path;
use std::pin::Pin;
use std::future::Future;

/// Trait for package managers to implement
pub trait PackageManager: Send + Sync {
    /// Returns the display name for this package manager
    fn name(&self) -> &str;
    
    /// Returns the emoji icon for this package manager
    fn icon(&self) -> &str;
    
    /// Detects if this package manager manages the given repository
    fn detect(&self, path: &Path) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;
    
    /// Gets package information from the repository
    fn get_info(&self, path: &Path) -> Pin<Box<dyn Future<Output = Option<PackageInfo>> + Send + '_>>;
    
    /// Publishes the package
    fn publish(&self, path: &Path, dry_run: bool) -> Pin<Box<dyn Future<Output = (bool, String)> + Send + '_>>;
}

/// Information about a detected package
#[derive(Clone, Debug)]
pub struct PackageInfo {
    pub manager_name: String,
    #[allow(dead_code)]
    pub name: String,
    pub version: String,
}

use std::sync::Arc;

/// Returns a list of all supported package managers
pub fn get_all_managers() -> Vec<Arc<dyn PackageManager>> {
    vec![
        Arc::new(npm::Npm),
        Arc::new(cargo::Cargo),
        Arc::new(pypi::PyPI),
    ]
}

/// Helper to detect package manager for a path (returns the first match)
/// This replaces the old `detect_package_manager` and `detect_package_manager_async`
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
pub enum PublishStatus {
    /// Package was successfully published
    Published,
    /// Package is already published (version exists)
    AlreadyPublished,
    /// Package was skipped (no package manager detected)
    #[allow(dead_code)]
    Skipped,
    /// An error occurred during publishing
    Error,
    /// Dry run completed successfully
    #[allow(dead_code)]
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