use async_trait::async_trait;
use std::path::Path;
use anyhow::Result;
use super::{PackageInfo, PackageManager};

/// Trait for package managers (npm, cargo, pypi, etc.)
#[async_trait]
pub trait PackageProvider: Send + Sync {
    /// Checks if the directory contains a package managed by this provider
    async fn detect(&self, path: &Path) -> bool;

    /// Gets information about the package
    async fn get_info(&self, path: &Path) -> Option<PackageInfo>;

    /// Publishes the package
    async fn publish(&self, path: &Path, dry_run: bool) -> Result<(bool, String)>;

    /// Returns the corresponding PackageManager enum variant
    fn manager_type(&self) -> PackageManager;
}
