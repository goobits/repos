//! Python package publishing functionality

use anyhow::Result;
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

use super::PackageInfo;

const PYTHON_OPERATION_TIMEOUT_SECS: u64 = 300; // 5 minutes for python operations

/// pyproject.toml structure (partial)
#[derive(Deserialize)]
struct PyProjectToml {
    project: Option<PyProject>,
}

#[derive(Deserialize)]
struct PyProject {
    name: String,
    version: String,
}

/// Gets package information from pyproject.toml or setup.py
pub async fn get_package_info(repo_path: &Path) -> Option<PackageInfo> {
    // Try pyproject.toml first
    let pyproject_path = repo_path.join("pyproject.toml");
    if pyproject_path.exists() {
        if let Ok(content) = tokio::fs::read_to_string(&pyproject_path).await {
            if let Ok(pyproject) = toml::from_str::<PyProjectToml>(&content) {
                if let Some(project) = pyproject.project {
                    return Some(PackageInfo {
                        manager: super::PackageManager::PyPI,
                        name: project.name,
                        version: project.version,
                    });
                }
            }
        }
    }

    // If pyproject.toml doesn't work, try to get info from setup.py by running python
    let setup_py_path = repo_path.join("setup.py");
    if setup_py_path.exists() {
        if let Ok(output) = Command::new("python")
            .args(&["-c", "import setuptools; print('OK')"])
            .current_dir(repo_path)
            .output()
            .await
        {
            if output.status.success() {
                // We can't easily extract name/version from setup.py without running it
                // Return a placeholder
                return Some(PackageInfo {
                    manager: super::PackageManager::PyPI,
                    name: "unknown".to_string(),
                    version: "unknown".to_string(),
                });
            }
        }
    }

    None
}

/// Publishes a Python package
/// Returns (success, message)
pub async fn publish(repo_path: &Path, dry_run: bool) -> (bool, String) {
    // First, check if twine is available
    let twine_check = Command::new("twine").arg("--version").output().await;

    if twine_check.is_err() {
        return (
            false,
            "twine not installed (run: pip install twine)".to_string(),
        );
    }

    // Build the package first
    let build_result = build_package(repo_path).await;
    if let Err(e) = build_result {
        return (false, format!("build failed: {}", e));
    }

    // Upload with twine
    let mut args = vec!["upload"];

    // Find the dist directory
    let dist_path = repo_path.join("dist");
    if !dist_path.exists() {
        return (false, "dist directory not found after build".to_string());
    }

    args.push("dist/*");

    if dry_run {
        // For dry-run, just check the packages
        args.clear();
        args.push("check");
        args.push("dist/*");
    }

    let timeout_duration = Duration::from_secs(PYTHON_OPERATION_TIMEOUT_SECS);

    let result = tokio::time::timeout(
        timeout_duration,
        Command::new("twine")
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
                let stdout = String::from_utf8_lossy(&output.stdout);
                let combined = format!("{}{}", stdout, stderr);
                let error_message = clean_python_error(&combined);

                // Check if it's an "already published" error
                if combined.contains("File already exists") || combined.contains("already exists") {
                    (true, "already published".to_string())
                } else {
                    (false, error_message)
                }
            }
        }
        Ok(Err(e)) => (false, format!("twine command failed: {}", e)),
        Err(_) => (false, "python operation timed out".to_string()),
    }
}

/// Builds a Python package
async fn build_package(repo_path: &Path) -> Result<()> {
    // Try using build module (modern approach)
    let build_result = Command::new("python")
        .args(&["-m", "build"])
        .current_dir(repo_path)
        .output()
        .await;

    if let Ok(output) = build_result {
        if output.status.success() {
            return Ok(());
        }
    }

    // Fallback to setup.py
    let setup_result = Command::new("python")
        .args(&["setup.py", "sdist", "bdist_wheel"])
        .current_dir(repo_path)
        .output()
        .await?;

    if setup_result.status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("Failed to build package"))
    }
}

/// Cleans up Python error messages to be more user-friendly
fn clean_python_error(error: &str) -> String {
    if error.contains("File already exists") {
        "already published".to_string()
    } else if error.contains("Invalid or non-existent authentication") {
        "not authenticated (configure ~/.pypirc)".to_string()
    } else if error.contains("403") {
        "permission denied (check PyPI permissions)".to_string()
    } else {
        // Return first meaningful line
        error
            .lines()
            .find(|line| !line.trim().is_empty() && !line.contains("Uploading"))
            .map(|line| line.trim().to_string())
            .unwrap_or_else(|| error.trim().to_string())
    }
}
