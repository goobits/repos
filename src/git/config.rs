//! Git user configuration management

use anyhow::Result;
use std::path::{Path, PathBuf};

use super::operations::{run_git, get_git_config, set_git_config};
use super::status::Status;

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
    Current(PathBuf),
    /// Interactive selection (prompts user to choose)
    Interactive,
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

/// Checks and optionally updates repository user configuration
/// Returns (status, message) indicating the result of the config operation
pub async fn check_repo_config(
    path: &Path,
    repo_name: &str,
    target_config: &UserConfig,
    command: &UserCommand,
) -> (Status, String) {
    use crate::commands::user::{prompt_for_config_resolution};

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

    // Determine if we should update based on command mode
    let should_update = match command {
        UserCommand::Force(_) => true,
        UserCommand::Interactive(_) => {
            // For interactive mode, prompt user for conflicts
            match prompt_for_config_resolution(repo_name, &current_config, target_config).await {
                Ok(update) => update,
                Err(_) => false, // Skip on error
            }
        }
        UserCommand::DryRun(_) => false, // Already handled above
    };

    if !should_update {
        return (Status::ConfigSkipped, "config unchanged".to_string());
    }

    // Apply configuration changes
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
    } else {
        (
            Status::ConfigUpdated,
            format!("updated: {}", updates.join(", ")),
        )
    }
}