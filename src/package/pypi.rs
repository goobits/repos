//! Python package publishing functionality

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

use super::{run_package_command, CommandEffect, PackageInfo, PackageManager, PackageManifest};

const PYTHON_OPERATION_TIMEOUT_SECS: u64 = 300; // 5 minutes for python operations

pub struct PyPI;

#[async_trait]
impl PackageManager for PyPI {
    fn name(&self) -> &str {
        "python"
    }

    fn icon(&self) -> &str {
        "📦"
    }

    async fn detect(&self, path: &Path) -> bool {
        tokio::fs::metadata(path.join("pyproject.toml"))
            .await
            .is_ok()
            || tokio::fs::metadata(path.join("setup.py")).await.is_ok()
    }

    async fn get_info(&self, path: &Path) -> Option<PackageInfo> {
        get_package_info_internal(path).await
    }

    async fn dependencies(&self, path: &Path) -> Vec<String> {
        get_pyproject(path)
            .await
            .ok()
            .flatten()
            .and_then(|project| project.project)
            .map(|project| project.dependencies)
            .unwrap_or_default()
    }

    async fn inspect_manifest(&self, path: &Path) -> Result<Option<PackageManifest>> {
        if let Some(manifest) = get_pyproject(path).await? {
            let project = manifest
                .project
                .ok_or_else(|| anyhow::anyhow!("pyproject.toml has no [project] metadata"))?;
            return Ok(Some(PackageManifest {
                dependencies: project.dependencies,
                info: PackageInfo {
                    manager_name: "python".to_string(),
                    name: project.name,
                    version: project.version,
                },
            }));
        }
        if path.join("setup.py").is_file() {
            anyhow::bail!(
                "setup.py metadata cannot be inspected safely; add static pyproject.toml metadata"
            );
        }
        Ok(None)
    }

    async fn publish(&self, path: &Path, dry_run: bool) -> (bool, String) {
        publish_internal(path, dry_run).await
    }
}

/// pyproject.toml structure (partial)
#[derive(Deserialize)]
struct PyProjectToml {
    project: Option<PyProject>,
}

#[derive(Deserialize)]
struct PyProject {
    name: String,
    version: String,
    #[serde(default)]
    dependencies: Vec<String>,
}

async fn get_pyproject(repo_path: &Path) -> Result<Option<PyProjectToml>> {
    let path = repo_path.join("pyproject.toml");
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    Ok(Some(toml::from_str(&content)?))
}

/// Gets package information from pyproject.toml or setup.py
async fn get_package_info_internal(repo_path: &Path) -> Option<PackageInfo> {
    // Try pyproject.toml first
    let pyproject_path = repo_path.join("pyproject.toml");
    if pyproject_path.exists() {
        if let Some(project) = get_pyproject(repo_path)
            .await
            .ok()
            .flatten()
            .and_then(|value| value.project)
        {
            return Some(PackageInfo {
                manager_name: "python".to_string(),
                name: project.name,
                version: project.version,
            });
        }
    }

    // If pyproject.toml doesn't work, try to get info from setup.py by running python
    let setup_py_path = repo_path.join("setup.py");
    if setup_py_path.exists() {
        let mut command = Command::new("python");
        command
            .args(["-c", "import setuptools; print('OK')"])
            .current_dir(repo_path);
        if let Ok(output) = run_package_command(
            command,
            Duration::from_secs(PYTHON_OPERATION_TIMEOUT_SECS),
            "python metadata check",
            CommandEffect::Local,
        )
        .await
        {
            if output.status.success() {
                // We can't easily extract name/version from setup.py without running it
                // Return a placeholder
                return Some(PackageInfo {
                    manager_name: "python".to_string(),
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
async fn publish_internal(repo_path: &Path, dry_run: bool) -> (bool, String) {
    publish_with_commands(
        repo_path,
        dry_run,
        Path::new("python"),
        Path::new("twine"),
        Duration::from_secs(PYTHON_OPERATION_TIMEOUT_SECS),
    )
    .await
}

async fn publish_with_commands(
    repo_path: &Path,
    dry_run: bool,
    python_program: &Path,
    twine_program: &Path,
    timeout_duration: Duration,
) -> (bool, String) {
    let mut twine_check = Command::new(twine_program);
    twine_check.arg("--version").current_dir(repo_path);
    match run_package_command(
        twine_check,
        timeout_duration,
        "twine version check",
        CommandEffect::Local,
    )
    .await
    {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            let detail = combined_output(&output);
            return (
                false,
                if detail.trim().is_empty() {
                    "twine unavailable (run: pip install twine)".to_string()
                } else {
                    format!("twine unavailable: {}", clean_python_error(&detail))
                },
            );
        }
        Err(error) => return (false, error.to_string()),
    }

    let output_dir = match tempfile::Builder::new().prefix("repos-pypi-").tempdir() {
        Ok(directory) => directory,
        Err(error) => {
            return (
                false,
                format!("could not create private build directory: {error}"),
            )
        }
    };
    let artifacts = match build_package(
        repo_path,
        output_dir.path(),
        python_program,
        timeout_duration,
    )
    .await
    {
        Ok(artifacts) => artifacts,
        Err(error) => return (false, format!("build failed: {error}")),
    };

    let mut twine = Command::new(twine_program);
    twine
        .arg(if dry_run { "check" } else { "upload" })
        .args(&artifacts)
        .current_dir(repo_path);
    let effect = if dry_run {
        CommandEffect::Local
    } else {
        CommandEffect::RegistryMutation
    };
    let operation = if dry_run {
        "twine artifact check"
    } else {
        "twine upload"
    };
    match run_package_command(twine, timeout_duration, operation, effect).await {
        Ok(output) if output.status.success() => (
            true,
            if dry_run { "dry-run ok" } else { "published" }.to_string(),
        ),
        Ok(output) => {
            let combined = combined_output(&output);
            if combined.contains("File already exists") || combined.contains("already exists") {
                (true, "already published".to_string())
            } else {
                (false, clean_python_error(&combined))
            }
        }
        Err(error) => (false, error.to_string()),
    }
}

/// Builds a Python package into an invocation-private directory and returns
/// only artifacts created by that build.
async fn build_package(
    repo_path: &Path,
    output_dir: &Path,
    python_program: &Path,
    timeout_duration: Duration,
) -> Result<Vec<PathBuf>> {
    let mut modern = Command::new(python_program);
    modern
        .args(["-m", "build", "--outdir"])
        .arg(output_dir)
        .current_dir(repo_path);
    let modern_output = run_package_command(
        modern,
        timeout_duration,
        "python package build",
        CommandEffect::Local,
    )
    .await?;
    if modern_output.status.success() {
        return collect_built_artifacts(output_dir).await;
    }

    if !repo_path.join("setup.py").is_file() {
        anyhow::bail!(
            "python -m build failed: {}",
            clean_python_error(&combined_output(&modern_output))
        );
    }

    let mut legacy = Command::new(python_program);
    legacy
        .arg("setup.py")
        .arg("sdist")
        .arg("--dist-dir")
        .arg(output_dir)
        .arg("bdist_wheel")
        .arg("--dist-dir")
        .arg(output_dir)
        .current_dir(repo_path);
    let legacy_output = run_package_command(
        legacy,
        timeout_duration,
        "legacy python package build",
        CommandEffect::Local,
    )
    .await?;
    if !legacy_output.status.success() {
        anyhow::bail!(
            "setup.py build failed: {}",
            clean_python_error(&combined_output(&legacy_output))
        );
    }
    collect_built_artifacts(output_dir).await
}

async fn collect_built_artifacts(output_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = tokio::fs::read_dir(output_dir)
        .await
        .with_context(|| format!("could not inspect {}", output_dir.display()))?;
    let mut artifacts = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_type().await?.is_file() {
            artifacts.push(entry.path());
        }
    }
    artifacts.sort();
    if artifacts.is_empty() {
        anyhow::bail!("package build produced no artifacts");
    }
    Ok(artifacts)
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
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
            .map_or_else(|| error.trim().to_string(), |line| line.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn write_executable(path: &Path, contents: &str) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(path, contents).expect("fake executable should be written");
        let mut permissions = std::fs::metadata(path)
            .expect("fake executable metadata")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions).expect("fake executable should be executable");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn upload_uses_only_artifacts_from_the_private_build_directory() {
        let root = tempfile::TempDir::new().expect("temporary directory");
        let repo = root.path().join("repo");
        let tools = root.path().join("tools");
        std::fs::create_dir_all(repo.join("dist")).expect("stale dist should be created");
        std::fs::create_dir(&tools).expect("tool directory should be created");
        let stale = repo.join("dist/stale.whl");
        std::fs::write(&stale, "stale").expect("stale artifact should be written");

        let python = tools.join("python");
        write_executable(
            &python,
            r#"#!/bin/sh
out=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --outdir|--dist-dir) out="$2"; shift 2 ;;
    *) shift ;;
  esac
done
test -n "$out" || exit 2
printf fresh > "$out/fresh.whl"
"#,
        );
        let twine = tools.join("twine");
        write_executable(
            &twine,
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  printf 'twine test\n'
  exit 0
fi
printf '%s\n' "$@" > "$0.args"
"#,
        );

        let result =
            publish_with_commands(&repo, false, &python, &twine, Duration::from_secs(2)).await;

        assert_eq!(result, (true, "published".to_string()));
        let arguments = std::fs::read_to_string(tools.join("twine.args")).expect("twine arguments");
        let lines = arguments.lines().collect::<Vec<_>>();
        assert_eq!(lines[0], "upload");
        assert_eq!(lines.len(), 2, "{arguments}");
        assert!(lines[1].ends_with("/fresh.whl"), "{arguments}");
        assert!(!arguments.contains(&stale.to_string_lossy().to_string()));
        assert!(!lines[1].starts_with(&repo.join("dist").to_string_lossy().to_string()));
    }
}
