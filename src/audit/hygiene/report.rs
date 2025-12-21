//! Hygiene reporting and statistics

use std::time::Duration;
use crate::utils::shorten_path;
use crate::core::config::PATH_DISPLAY_WIDTH;

/// Status for hygiene check results
#[derive(Clone, Debug)]
pub enum HygieneStatus {
    Clean,      // No hygiene violations found
    Violations, // Hygiene violations detected
    Error,      // Scan failed
}

impl HygieneStatus {
    pub fn symbol(&self) -> &str {
        match self {
            HygieneStatus::Clean => "🟢",
            HygieneStatus::Violations => "🟡",
            HygieneStatus::Error => "🟠",
        }
    }

    pub fn text(&self) -> &str {
        match self {
            HygieneStatus::Clean => "clean",
            HygieneStatus::Violations => "violations",
            HygieneStatus::Error => "failed",
        }
    }
}

/// Type of hygiene violation
#[derive(Clone, Debug)]
pub enum ViolationType {
    GitignoreViolation,  // File tracked but matches .gitignore
    UniversalBadPattern, // File matches universal bad patterns
    LargeFile,           // File is unusually large
}

/// Individual hygiene violation
#[derive(Clone, Debug)]
pub struct HygieneViolation {
    pub file_path: String,
    pub violation_type: ViolationType,
    pub size_bytes: Option<u64>,
}

/// Statistics for hygiene scanning results
#[derive(Clone, Default)]
pub struct HygieneStatistics {
    clean_repos: u32,
    repos_with_violations: u32,
    total_violations: u32,
    gitignore_violations: u32,
    universal_violations: u32,
    large_files: u32,
    error_repos: u32,
    failed_repos: Vec<(String, String, String)>, // (repo_name, repo_path, error_message)
    violation_repos: Vec<(String, String, Vec<HygieneViolation>)>, // (repo_name, repo_path, violations)
}

impl HygieneStatistics {
    #[must_use] 
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the violation repositories for fix operations
    #[must_use] 
    pub fn get_violation_repos(&self) -> Vec<(String, String, Vec<HygieneViolation>)> {
        self.violation_repos.clone()
    }

    pub fn update(
        &mut self,
        repo_name: &str,
        repo_path: &str,
        status: &HygieneStatus,
        message: &str,
        violations: Vec<HygieneViolation>,
    ) {
        match status {
            HygieneStatus::Clean => self.clean_repos += 1,
            HygieneStatus::Violations => {
                self.repos_with_violations += 1;
                let violation_count = violations.len() as u32;
                self.total_violations += violation_count;

                // Count by type
                for violation in &violations {
                    match violation.violation_type {
                        ViolationType::GitignoreViolation => self.gitignore_violations += 1,
                        ViolationType::UniversalBadPattern => self.universal_violations += 1,
                        ViolationType::LargeFile => self.large_files += 1,
                    }
                }

                self.violation_repos.push((
                    repo_name.to_string(),
                    repo_path.to_string(),
                    violations,
                ));
            }
            HygieneStatus::Error => {
                self.error_repos += 1;
                self.failed_repos.push((
                    repo_name.to_string(),
                    repo_path.to_string(),
                    message.to_string(),
                ));
            }
        }
    }

    #[must_use] 
    pub fn generate_summary(&self, _total_repos: usize, duration: Duration) -> String {
        let duration_secs = duration.as_secs_f64();

        if self.error_repos > 0 {
            format!(
                "✅ Completed in {:.1}s • {} clean • {} with violations • {} failed",
                duration_secs, self.clean_repos, self.repos_with_violations, self.error_repos
            )
        } else {
            format!(
                "✅ Completed in {:.1}s • {} clean • {} with violations",
                duration_secs, self.clean_repos, self.repos_with_violations
            )
        }
    }

    #[must_use] 
    pub fn generate_detailed_summary(&self) -> String {
        let mut lines = Vec::new();

        // Repos with violations get priority
        if !self.violation_repos.is_empty() {
            lines.push(format!(
                "🟡 HYGIENE VIOLATIONS ({})",
                self.violation_repos.len()
            ));
            for (i, (repo_name, repo_path, violations)) in self.violation_repos.iter().enumerate() {
                let tree_char = if i == self.violation_repos.len() - 1 {
                    "└─"
                } else {
                    "├─"
                };
                let short_path = shorten_path(repo_path, PATH_DISPLAY_WIDTH);
                let violation_summary = format!(
                    "{} violations ({} gitignore, {} patterns, {} large)",
                    violations.len(),
                    violations
                        .iter()
                        .filter(|v| matches!(v.violation_type, ViolationType::GitignoreViolation))
                        .count(),
                    violations
                        .iter()
                        .filter(|v| matches!(v.violation_type, ViolationType::UniversalBadPattern))
                        .count(),
                    violations
                        .iter()
                        .filter(|v| matches!(v.violation_type, ViolationType::LargeFile))
                        .count()
                );
                lines.push(format!(
                    "   {tree_char} {repo_name:20} {short_path:30} # {violation_summary}"
                ));
            }
            lines.push(String::new()); // Add blank line
        }

        // Failed repos
        if !self.failed_repos.is_empty() {
            lines.push(format!(
                "🟠 FAILED HYGIENE SCANS ({})",
                self.failed_repos.len()
            ));
            for (i, (repo_name, repo_path, error)) in self.failed_repos.iter().enumerate() {
                let tree_char = if i == self.failed_repos.len() - 1 {
                    "└─"
                } else {
                    "├─"
                };
                let short_path = shorten_path(repo_path, PATH_DISPLAY_WIDTH);
                lines.push(format!(
                    "   {tree_char} {repo_name:20} {short_path:30} # {error}"
                ));
            }
        }

        // Remove trailing blank line if it exists
        if lines.last() == Some(&String::new()) {
            lines.pop();
        }

        lines.join("\n")
    }
}
