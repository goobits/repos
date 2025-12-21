//! Package management and publishing functionality

pub mod cargo;
pub mod npm;
pub mod pypi;

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
#[allow(dead_code)]
pub struct PackageInfo {
    pub manager: PackageManager,
    pub name: String,
    pub version: String,
}

/// Detects the package manager for a given repository path (synchronous version)
/// Returns None if no package manager is detected
pub fn detect_package_manager(repo_path: &Path) -> Option<PackageManager> {
    if repo_path.join("package.json").exists() {
        Some(PackageManager::Npm)
    } else if repo_path.join("Cargo.toml").exists() {
        Some(PackageManager::Cargo)
    } else if repo_path.join("pyproject.toml").exists() || repo_path.join("setup.py").exists() {
        Some(PackageManager::PyPI)
    } else {
        None
    }
}

/// Detects the package manager for a given repository path (async version using tokio::fs)
/// Returns None if no package manager is detected
/// This is significantly faster when called in parallel on many repositories
pub async fn detect_package_manager_async(repo_path: &Path) -> Option<PackageManager> {
    use tokio::fs;

    // Check for package.json (npm)
    if fs::metadata(repo_path.join("package.json")).await.is_ok() {
        return Some(PackageManager::Npm);
    }

    // Check for Cargo.toml (cargo)
    if fs::metadata(repo_path.join("Cargo.toml")).await.is_ok() {
        return Some(PackageManager::Cargo);
    }

    // Check for pyproject.toml or setup.py (PyPI)
    if fs::metadata(repo_path.join("pyproject.toml")).await.is_ok()
        || fs::metadata(repo_path.join("setup.py")).await.is_ok()
    {
        return Some(PackageManager::PyPI);
    }

    None
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_npm_package() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().expect("Failed to create temp directory for test");

        // Create package.json
        std::fs::write(
            temp_dir.path().join("package.json"),
            r#"{"name": "test", "version": "1.0.0"}"#,
        )
        .expect("Failed to write package.json test file");

        let manager = detect_package_manager(temp_dir.path());
        assert_eq!(manager, Some(PackageManager::Npm));
    }

    #[test]
    fn test_detect_cargo_package() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().expect("Failed to create temp directory for test");

        // Create Cargo.toml
        std::fs::write(
            temp_dir.path().join("Cargo.toml"),
            r#"[package]
name = "test"
version = "1.0.0"
"#,
        )
        .expect("Failed to write Cargo.toml test file");

        let manager = detect_package_manager(temp_dir.path());
        assert_eq!(manager, Some(PackageManager::Cargo));
    }

    #[test]
    fn test_detect_pypi_package_pyproject() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().expect("Failed to create temp directory for test");

        // Create pyproject.toml
        std::fs::write(
            temp_dir.path().join("pyproject.toml"),
            r#"[project]
name = "test"
version = "1.0.0"
"#,
        )
        .expect("Failed to write pyproject.toml test file");

        let manager = detect_package_manager(temp_dir.path());
        assert_eq!(manager, Some(PackageManager::PyPI));
    }

    #[test]
    fn test_detect_pypi_package_setup_py() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().expect("Failed to create temp directory for test");

        // Create setup.py
        std::fs::write(
            temp_dir.path().join("setup.py"),
            r#"from setuptools import setup
setup(name="test", version="1.0.0")
"#,
        )
        .expect("Failed to write setup.py test file");

        let manager = detect_package_manager(temp_dir.path());
        assert_eq!(manager, Some(PackageManager::PyPI));
    }

    #[test]
    fn test_detect_no_package() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().expect("Failed to create temp directory for test");

        // No package files
        let manager = detect_package_manager(temp_dir.path());
        assert_eq!(manager, None);
    }

    #[test]
    fn test_npm_priority_over_others() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().expect("Failed to create temp directory for test");

        // Create both package.json and Cargo.toml
        std::fs::write(
            temp_dir.path().join("package.json"),
            r#"{"name": "test", "version": "1.0.0"}"#,
        )
        .expect("Failed to write package.json test file");
        std::fs::write(
            temp_dir.path().join("Cargo.toml"),
            r#"[package]
name = "test"
version = "1.0.0"
"#,
        )
        .expect("Failed to write Cargo.toml test file");

        // Should prefer npm (checked first)
        let manager = detect_package_manager(temp_dir.path());
        assert_eq!(manager, Some(PackageManager::Npm));
    }

    #[tokio::test]
    async fn test_detect_package_manager_async_npm() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().expect("Failed to create temp directory for async test");

        std::fs::write(temp_dir.path().join("package.json"), r#"{"name": "test"}"#)
            .expect("Failed to write package.json test file");

        let manager = detect_package_manager_async(temp_dir.path()).await;
        assert_eq!(manager, Some(PackageManager::Npm));
    }

    #[tokio::test]
    async fn test_detect_package_manager_async_cargo() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().expect("Failed to create temp directory for async test");

        std::fs::write(
            temp_dir.path().join("Cargo.toml"),
            r#"[package]
name = "test"
"#,
        )
        .expect("Failed to write Cargo.toml test file");

        let manager = detect_package_manager_async(temp_dir.path()).await;
        assert_eq!(manager, Some(PackageManager::Cargo));
    }

    #[tokio::test]
    async fn test_detect_package_manager_async_none() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().expect("Failed to create temp directory for async test");

        let manager = detect_package_manager_async(temp_dir.path()).await;
        assert_eq!(manager, None);
    }

    #[tokio::test]
    async fn test_async_vs_sync_consistency() {
        use tempfile::TempDir;
        let temp_dir = TempDir::new().expect("Failed to create temp directory for async test");

        // Test with npm
        std::fs::write(temp_dir.path().join("package.json"), r#"{"name": "test"}"#)
            .expect("Failed to write package.json test file");

        let sync_result = detect_package_manager(temp_dir.path());
        let async_result = detect_package_manager_async(temp_dir.path()).await;

        assert_eq!(
            sync_result, async_result,
            "Sync and async should return same result"
        );
    }
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
