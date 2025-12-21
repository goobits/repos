//! Test fixtures and builders

use anyhow::Result;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

use super::git::{add_git_remote, create_test_commit, setup_git_repo};

/// A test repository with automatic cleanup
#[allow(dead_code)]
pub struct TestRepo {
    pub temp_dir: TempDir,
    pub name: String,
}

impl TestRepo {
    /// Get the path to the repository
    pub fn path(&self) -> &Path {
        self.temp_dir.path()
    }

    /// Create a new file in the repository
    pub fn create_file(&self, name: &str, content: &str) -> Result<PathBuf> {
        let file_path = self.path().join(name);
        std::fs::write(&file_path, content)?;
        Ok(file_path)
    }

    /// Create a package.json file for npm testing
    pub fn create_package_json(&self, name: &str, version: &str) -> Result<()> {
        let content = format!(
            r#"{{
  "name": "{}",
  "version": "{}",
  "description": "Test package",
  "main": "index.js"
}}"#,
            name, version
        );
        self.create_file("package.json", &content)?;
        Ok(())
    }

    /// Create a Cargo.toml file for Rust testing
    pub fn create_cargo_toml(&self, name: &str, version: &str) -> Result<()> {
        let content = format!(
            r#"[package]
name = "{}"
version = "{}"
edition = "2021"

[dependencies]
"#,
            name, version
        );
        self.create_file("Cargo.toml", &content)?;
        Ok(())
    }

    /// Create a pyproject.toml file for Python testing
    pub fn create_pyproject_toml(&self, name: &str, version: &str) -> Result<()> {
        let content = format!(
            r#"[project]
name = "{}"
version = "{}"
description = "Test package"
"#,
            name, version
        );
        self.create_file("pyproject.toml", &content)?;
        Ok(())
    }

    /// Commit all changes in the repository
    pub fn commit_all(&self, message: &str) -> Result<()> {
        use std::process::Command;

        Command::new("git")
            .args(["add", "."])
            .current_dir(self.path())
            .output()?;

        let result = Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(self.path())
            .output()?;

        if !result.status.success() {
            anyhow::bail!(
                "Failed to commit: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }

        Ok(())
    }
}

/// Builder for creating test repositories
pub struct TestRepoBuilder {
    name: String,
    with_remote: Option<String>,
    with_package: Option<PackageType>,
    with_commits: usize,
}

#[derive(Clone)]
#[allow(dead_code)]
pub enum PackageType {
    Npm { name: String, version: String },
    Cargo { name: String, version: String },
    PyPI { name: String, version: String },
}

impl TestRepoBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            with_remote: None,
            with_package: None,
            with_commits: 1,
        }
    }

    #[allow(dead_code)]
    pub fn with_github_remote(mut self, url: impl Into<String>) -> Self {
        self.with_remote = Some(url.into());
        self
    }

    #[allow(dead_code)]
    pub fn with_npm_package(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.with_package = Some(PackageType::Npm {
            name: name.into(),
            version: version.into(),
        });
        self
    }

    #[allow(dead_code)]
    pub fn with_cargo_package(
        mut self,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        self.with_package = Some(PackageType::Cargo {
            name: name.into(),
            version: version.into(),
        });
        self
    }

    #[allow(dead_code)]
    pub fn with_python_package(
        mut self,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        self.with_package = Some(PackageType::PyPI {
            name: name.into(),
            version: version.into(),
        });
        self
    }

    #[allow(dead_code)]
    pub fn with_commits(mut self, count: usize) -> Self {
        self.with_commits = count;
        self
    }

    pub fn build(self) -> Result<TestRepo> {
        let temp_dir = TempDir::new()?;
        setup_git_repo(temp_dir.path())?;

        // Create initial commit
        create_test_commit(
            temp_dir.path(),
            "README.md",
            "# Test Repo",
            "Initial commit",
        )?;

        // Add remote if specified
        if let Some(remote_url) = self.with_remote {
            add_git_remote(temp_dir.path(), "origin", &remote_url)?;
        }

        let repo = TestRepo {
            temp_dir,
            name: self.name,
        };

        // Create package files if specified
        if let Some(package_type) = self.with_package {
            match package_type {
                PackageType::Npm { name, version } => {
                    repo.create_package_json(&name, &version)?;
                }
                PackageType::Cargo { name, version } => {
                    repo.create_cargo_toml(&name, &version)?;
                }
                PackageType::PyPI { name, version } => {
                    repo.create_pyproject_toml(&name, &version)?;
                }
            }
            repo.commit_all("Add package manifest")?;
        }

        // Create additional commits if specified
        for i in 2..=self.with_commits {
            create_test_commit(
                repo.path(),
                &format!("file{}.txt", i),
                &format!("Content {}", i),
                &format!("Commit {}", i),
            )?;
        }

        Ok(repo)
    }
}
