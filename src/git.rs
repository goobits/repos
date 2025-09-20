//! Git operations and configuration management
//!
//! This module handles all interactions with git repositories including:
//! - Low-level git command execution
//! - User configuration read/write operations
//! - Config validation and conflict detection

use anyhow::Result;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

// Timeout constants
const GIT_OPERATION_TIMEOUT_SECS: u64 = 180; // 3 minutes per repository

// Git command arguments
const GIT_DIFF_INDEX_ARGS: &[&str] = &["diff-index", "--quiet", "HEAD", "--"];
const GIT_REMOTE_ARGS: &[&str] = &["remote"];
const GIT_REV_PARSE_HEAD_ARGS: &[&str] = &["rev-parse", "--abbrev-ref", "HEAD"];
const GIT_FETCH_ARGS: &[&str] = &["fetch", "--quiet"];
const GIT_PUSH_ARGS: &[&str] = &["push"];
const GIT_CONFIG_GET_ARGS: &[&str] = &["config", "--get"];

// Status messages
const DETACHED_HEAD_BRANCH: &str = "HEAD";
const STATUS_NO_REMOTE: &str = "no remote";
const STATUS_DETACHED_HEAD: &str = "detached HEAD";
const STATUS_NO_UPSTREAM: &str = "no tracking";
const STATUS_SYNCED: &str = "up to date";

/// Represents user configuration (name and email) to sync across repositories
#[derive(Clone, Debug)]
pub struct UserConfig {
    pub name: Option<String>,
    pub email: Option<String>,
}

impl UserConfig {
    pub fn new(name: Option<String>, email: Option<String>) -> Self {
        Self { name, email }
    }

    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.email.is_none()
    }
}

/// Configuration source for determining user config values
#[derive(Clone)]
pub enum ConfigSource {
    /// Use provided name/email values
    Explicit(UserConfig),
    /// Use global git config as source
    Global,
    /// Use current repository's config as source
    Current(std::path::PathBuf),
}

/// Mode of operation for the user config command
#[derive(Clone)]
pub enum UserCommand {
    /// Interactive mode - detect conflicts and prompt for resolution
    Interactive(ConfigSource),
    /// Force mode - overwrite all configs without prompting
    Force(ConfigSource),
    /// Dry run mode - show what would be changed without making changes
    DryRun(ConfigSource),
}

/// CLI arguments for the user subcommand
#[derive(Clone)]
pub struct UserArgs {
    pub command: UserCommand,
}

/// Repository synchronization status
#[derive(Clone)]
pub enum Status {
    /// Repository is already up to date with remote
    Synced,
    /// Repository had commits that were successfully pushed
    Pushed,
    /// Repository was skipped (no remote, detached HEAD, etc.)
    Skip,
    /// Repository has no upstream tracking branch
    NoUpstream,
    /// Repository has no remote configured
    NoRemote,
    /// An error occurred during synchronization
    Error,
    /// Config was already synced
    ConfigSynced,
    /// Config was updated
    ConfigUpdated,
    /// Config operation was skipped
    ConfigSkipped,
    /// Config operation failed
    ConfigError,
}

impl Status {
    /// Returns the emoji symbol for this status
    pub fn symbol(&self) -> &str {
        match self {
            Status::Synced | Status::Pushed | Status::ConfigSynced | Status::ConfigUpdated => "🟢",
            Status::Skip | Status::NoRemote | Status::ConfigSkipped => "🟠",
            Status::NoUpstream => "🟡",
            Status::Error | Status::ConfigError => "🔴",
        }
    }

    /// Returns the text representation of this status
    pub fn text(&self) -> &str {
        match self {
            Status::Synced => "synced",
            Status::Pushed => "pushed",
            Status::Skip => "skip",
            Status::NoUpstream => "no-upstream",
            Status::NoRemote => "skip",
            Status::Error => "failed",
            Status::ConfigSynced => "config-ok",
            Status::ConfigUpdated => "config-updated",
            Status::ConfigSkipped => "config-skip",
            Status::ConfigError => "config-failed",
        }
    }
}

/// Runs a git command in the specified directory with a timeout
/// Returns (success, stdout, stderr)
pub async fn run_git(path: &Path, args: &[&str]) -> Result<(bool, String, String)> {
    let timeout_duration = Duration::from_secs(GIT_OPERATION_TIMEOUT_SECS);

    let result = tokio::time::timeout(
        timeout_duration,
        Command::new("git").args(args).current_dir(path).output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => Ok((
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        )),
        Ok(Err(e)) => Err(e.into()),
        Err(_) => Err(anyhow::anyhow!(
            "Git operation timed out after {} seconds",
            GIT_OPERATION_TIMEOUT_SECS
        )),
    }
}

/// Reads a git config value from the specified repository
/// Returns the config value if it exists, None if not found
pub async fn get_git_config(path: &Path, key: &str) -> Result<Option<String>> {
    let mut args = Vec::from(GIT_CONFIG_GET_ARGS);
    args.push(key);

    match run_git(path, &args).await {
        Ok((true, value, _)) => {
            if value.is_empty() {
                Ok(None)
            } else {
                Ok(Some(value))
            }
        }
        Ok((false, _, _)) => Ok(None), // Key not found
        Err(e) => Err(e),
    }
}

/// Sets a git config value in the specified repository (local scope)
/// Returns success status
pub async fn set_git_config(path: &Path, key: &str, value: &str) -> Result<bool> {
    let args = vec!["config", key, value];

    match run_git(path, &args).await {
        Ok((success, _, _)) => Ok(success),
        Err(e) => Err(e),
    }
}

/// Gets the current user config (name and email) from a repository
pub async fn get_current_user_config(path: &Path) -> (Option<String>, Option<String>) {
    let name = get_git_config(path, "user.name").await.unwrap_or(None);
    let email = get_git_config(path, "user.email").await.unwrap_or(None);
    (name, email)
}

/// Gets the global user config (name and email)
pub async fn get_global_user_config() -> (Option<String>, Option<String>) {
    // Use a temporary directory for global config access
    let temp_dir = std::env::temp_dir();

    let name = match run_git(&temp_dir, &["config", "--global", "--get", "user.name"]).await {
        Ok((true, value, _)) if !value.is_empty() => Some(value),
        _ => None,
    };

    let email = match run_git(&temp_dir, &["config", "--global", "--get", "user.email"]).await {
        Ok((true, value, _)) if !value.is_empty() => Some(value),
        _ => None,
    };

    (name, email)
}

/// Validates user config values according to basic requirements
pub fn validate_user_config(config: &UserConfig) -> Result<()> {
    if let Some(name) = &config.name {
        if name.trim().is_empty() {
            return Err(anyhow::anyhow!("User name cannot be empty"));
        }
    }

    if let Some(email) = &config.email {
        let email = email.trim();
        if email.is_empty() {
            return Err(anyhow::anyhow!("User email cannot be empty"));
        }
        // Basic email validation - must contain @ and at least one dot after @
        if !email.contains('@') || !email.split('@').nth(1).unwrap_or("").contains('.') {
            return Err(anyhow::anyhow!("Invalid email format: {}", email));
        }
    }

    Ok(())
}

/// Checks a git repository and attempts to push any unpushed commits
/// Returns (status, message, has_uncommitted_changes)
pub async fn check_repo(path: &Path, force_push: bool) -> (Status, String, bool) {
    // Check uncommitted changes
    let has_uncommitted_changes = !run_git(path, GIT_DIFF_INDEX_ARGS)
        .await
        .map(|(success, _, _)| success)
        .unwrap_or(false);

    // Check if repository has any remotes configured
    if let Ok((true, remotes, _)) = run_git(path, GIT_REMOTE_ARGS).await {
        if remotes.is_empty() {
            return (
                Status::NoRemote,
                STATUS_NO_REMOTE.to_string(),
                has_uncommitted_changes,
            );
        }
    } else {
        return (
            Status::NoRemote,
            STATUS_NO_REMOTE.to_string(),
            has_uncommitted_changes,
        );
    }

    // Get current branch
    let current_branch = match run_git(path, GIT_REV_PARSE_HEAD_ARGS).await {
        Ok((true, branch_name, _)) if branch_name != DETACHED_HEAD_BRANCH => branch_name,
        _ => {
            return (
                Status::Skip,
                STATUS_DETACHED_HEAD.to_string(),
                has_uncommitted_changes,
            )
        }
    };

    // Check if current branch has an upstream configured
    if !run_git(
        path,
        &[
            "rev-parse",
            "--abbrev-ref",
            &format!("{}@{{upstream}}", current_branch),
        ],
    )
    .await
    .map(|(success, _, _)| success)
    .unwrap_or(false)
    {
        if force_push {
            // Force push: set up upstream and push
            match run_git(path, &["push", "-u", "origin", &current_branch]).await {
                Ok((true, _, _)) => {
                    return (
                        Status::Pushed,
                        format!("{} (upstream set)", current_branch),
                        has_uncommitted_changes,
                    );
                }
                Ok((false, _, err)) => {
                    return (
                        Status::Error,
                        crate::core::clean_error_message(&format!(
                            "upstream setup failed: {}",
                            err
                        )),
                        has_uncommitted_changes,
                    );
                }
                Err(e) => {
                    return (
                        Status::Error,
                        crate::core::clean_error_message(&format!("upstream setup error: {}", e)),
                        has_uncommitted_changes,
                    );
                }
            }
        } else {
            return (
                Status::NoUpstream,
                format!("{} ({})", current_branch, STATUS_NO_UPSTREAM),
                has_uncommitted_changes,
            );
        }
    }

    // Fetch latest changes from remote
    if let Ok((false, _, err)) = run_git(path, GIT_FETCH_ARGS).await {
        return (
            Status::Error,
            crate::core::clean_error_message(&format!("fetch failed: {}", err)),
            has_uncommitted_changes,
        );
    }

    // Count commits that are ahead of upstream
    let unpushed_commits = run_git(
        path,
        &[
            "rev-list",
            "--count",
            &format!("{}@{{upstream}}..HEAD", current_branch),
        ],
    )
    .await
    .ok()
    .and_then(|(success, count, _)| {
        if success {
            count.parse::<u32>().ok()
        } else {
            None
        }
    })
    .unwrap_or(0);

    if unpushed_commits > 0 {
        // Attempt to push the unpushed commits
        match run_git(path, GIT_PUSH_ARGS).await {
            Ok((true, _, _)) => (
                Status::Pushed,
                format!("{} commits pushed", unpushed_commits),
                has_uncommitted_changes,
            ),
            Ok((false, _, err)) => (
                Status::Error,
                crate::core::clean_error_message(&err),
                has_uncommitted_changes,
            ),
            Err(e) => (
                Status::Error,
                crate::core::clean_error_message(&format!("push error: {}", e)),
                has_uncommitted_changes,
            ),
        }
    } else {
        (
            Status::Synced,
            STATUS_SYNCED.to_string(),
            has_uncommitted_changes,
        )
    }
}

/// Checks and syncs user config for a repository
/// Returns (status, message)
pub async fn check_repo_config(
    path: &Path,
    repo_name: &str,
    target_config: &UserConfig,
    command: &UserCommand,
) -> (Status, String) {
    // Get current config
    let (current_name, current_email) = get_current_user_config(path).await;
    let current_config = UserConfig::new(current_name, current_email);

    // Check if config needs updating
    let name_needs_update = match (&current_config.name, &target_config.name) {
        (Some(current), Some(target)) => current != target,
        (None, Some(_)) => true,
        _ => false,
    };

    let email_needs_update = match (&current_config.email, &target_config.email) {
        (Some(current), Some(target)) => current != target,
        (None, Some(_)) => true,
        _ => false,
    };

    if !name_needs_update && !email_needs_update {
        return (Status::ConfigSynced, "config synced".to_string());
    }

    // Handle dry run mode
    if matches!(command, UserCommand::DryRun(_)) {
        let mut changes = Vec::new();
        if name_needs_update {
            if let Some(target_name) = &target_config.name {
                changes.push(format!("name → {}", target_name));
            }
        }
        if email_needs_update {
            if let Some(target_email) = &target_config.email {
                changes.push(format!("email → {}", target_email));
            }
        }
        return (
            Status::ConfigSkipped,
            format!("would update: {}", changes.join(", ")),
        );
    }

    // Handle interactive mode - check for conflicts
    let should_update = match command {
        UserCommand::Force(_) => true,
        UserCommand::Interactive(_) => {
            // If there are existing values that would be changed, prompt for confirmation
            if (current_config.name.is_some() && name_needs_update)
                || (current_config.email.is_some() && email_needs_update)
            {
                match crate::user_command::prompt_for_config_resolution(
                    repo_name,
                    &current_config,
                    target_config,
                )
                .await
                {
                    Ok(update) => update,
                    Err(_) => false,
                }
            } else {
                // No existing config to conflict with, safe to update
                true
            }
        }
        UserCommand::DryRun(_) => unreachable!(), // Already handled above
    };

    if !should_update {
        return (Status::ConfigSkipped, "config skipped".to_string());
    }

    // Apply the config changes
    let mut updates = Vec::new();
    let mut errors = Vec::new();

    if name_needs_update {
        if let Some(target_name) = &target_config.name {
            match set_git_config(path, "user.name", target_name).await {
                Ok(true) => updates.push("name"),
                Ok(false) | Err(_) => errors.push("name"),
            }
        }
    }

    if email_needs_update {
        if let Some(target_email) = &target_config.email {
            match set_git_config(path, "user.email", target_email).await {
                Ok(true) => updates.push("email"),
                Ok(false) | Err(_) => errors.push("email"),
            }
        }
    }

    if !errors.is_empty() {
        (
            Status::ConfigError,
            format!("failed to update: {}", errors.join(", ")),
        )
    } else if !updates.is_empty() {
        (
            Status::ConfigUpdated,
            format!("updated: {}", updates.join(", ")),
        )
    } else {
        (Status::ConfigSynced, "config synced".to_string())
    }
}
