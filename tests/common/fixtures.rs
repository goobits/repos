//! Test repository fixtures

use anyhow::Result;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

use super::git::{create_test_commit, setup_git_repo};

/// A test repository with automatic cleanup
pub struct TestRepo {
    temp_dir: TempDir,
}

impl TestRepo {
    pub fn new() -> Result<Self> {
        let temp_dir = TempDir::new()?;
        setup_git_repo(temp_dir.path())?;
        create_test_commit(
            temp_dir.path(),
            "README.md",
            "# Test Repo",
            "Initial commit",
        )?;
        Ok(Self { temp_dir })
    }

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
