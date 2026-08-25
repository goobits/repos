//! Git user configuration management

use anyhow::Result;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use super::operations::set_git_config;
use super::runner::run_git_raw;
use super::status::Status;

/// Type alias for the interactive prompt function
/// Takes (`repo_name`, `current_config`, `target_config`) and returns whether to apply changes
pub type PromptFn = Box<
    dyn Fn(&str, &UserConfig, &UserConfig) -> Pin<Box<dyn Future<Output = Result<bool>> + Send>>
        + Send
        + Sync,
>;

/// Represents user configuration (name and email) to sync across repositories
#[derive(Clone, Debug)]
pub struct UserConfig {
    pub name: Option<String>,
    pub email: Option<String>,
}

impl UserConfig {
    #[must_use]
    pub fn new(name: Option<String>, email: Option<String>) -> Self {
        Self { name, email }
    }

    #[must_use]
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

/// Mode of operation for the config command
#[derive(Clone)]
pub enum ConfigCommand {
    /// Interactive mode - detect conflicts and prompt for resolution
    Interactive(ConfigSource),
    /// Apply mode - overwrite all configs without prompting
    Force(ConfigSource),
    /// Dry run mode - show what would be changed without making changes
    DryRun(ConfigSource),
}

/// CLI arguments for the config subcommand
#[derive(Clone)]
pub struct ConfigArgs {
    pub command: ConfigCommand,
}

/// Gets the current user config (name and email) from a repository
pub async fn get_current_user_config(path: &Path) -> Result<(Option<String>, Option<String>)> {
    get_user_config(path, false).await
}

/// Gets the global user config (name and email)
pub async fn get_global_user_config() -> Result<(Option<String>, Option<String>)> {
    // Use a temporary directory for global config access
    let temp_dir = std::env::temp_dir();

    get_user_config(&temp_dir, true).await
}

async fn get_user_config(path: &Path, global: bool) -> Result<(Option<String>, Option<String>)> {
    let args = if global {
        vec![
            "config",
            "--global",
            "--null",
            "--get-regexp",
            r"^(user\.name|user\.email)$",
        ]
    } else {
        vec![
            "config",
            "--null",
            "--get-regexp",
            r"^(user\.name|user\.email)$",
        ]
    };
    let output = run_git_raw(path, &args).await?;
    if output.success() {
        return parse_user_config(&output.stdout);
    }
    if output.exit_code == Some(1) && output.stderr.is_empty() {
        return Ok((None, None));
    }
    let stderr = output.stderr_text();
    anyhow::bail!(
        "{}",
        if stderr.is_empty() {
            format!(
                "git config inspection failed with exit code {:?}",
                output.exit_code
            )
        } else {
            stderr
        }
    )
}

fn parse_user_config(output: &[u8]) -> Result<(Option<String>, Option<String>)> {
    let mut name = None;
    let mut email = None;
    for record in output.split(|byte| *byte == 0) {
        if record.is_empty() {
            continue;
        }
        let record = std::str::from_utf8(record)?;
        let (key, value) = record
            .split_once('\n')
            .ok_or_else(|| anyhow::anyhow!("git config returned a malformed user record"))?;
        if key.eq_ignore_ascii_case("user.name") {
            name = Some(value.to_string());
        } else if key.eq_ignore_ascii_case("user.email") {
            email = Some(value.to_string());
        }
    }
    Ok((name, email))
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
            return Err(anyhow::anyhow!("Invalid email format: {email}"));
        }
    }

    Ok(())
}

/// Checks and optionally updates git configuration for a repository
///
/// # Parameters
/// - `path`: Path to the repository
/// - `repo_name`: Display name of the repository
/// - `target_config`: Desired configuration values
/// - `command`: Config command mode (Interactive, Apply, or `DryRun`)
/// - `prompt_fn`: Optional function to prompt user for interactive mode conflicts
///
/// Returns `(Status, message)` tuple indicating the result
pub async fn check_repo_config(
    path: &Path,
    repo_name: &str,
    target_config: &UserConfig,
    command: &ConfigCommand,
    prompt_fn: Option<&PromptFn>,
) -> (Status, String) {
    // Get current config
    let (current_name, current_email) = match get_current_user_config(path).await {
        Ok(config) => config,
        Err(error) => {
            return (
                Status::ConfigError,
                format!("config inspection failed: {error}"),
            )
        }
    };
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
    if matches!(command, ConfigCommand::DryRun(_)) {
        let mut changes = Vec::new();
        if name_needs_update {
            if let Some(target_name) = &target_config.name {
                changes.push(format!("name → {target_name}"));
            }
        }
        if email_needs_update {
            if let Some(target_email) = &target_config.email {
                changes.push(format!("email → {target_email}"));
            }
        }
        return (
            Status::ConfigSkipped,
            format!("would update: {}", changes.join(", ")),
        );
    }

    // Determine if we should update based on command mode
    let should_update = match command {
        ConfigCommand::Force(_) => true,
        ConfigCommand::Interactive(_) => {
            // For interactive mode, use provided prompt function or default to false
            if let Some(prompt) = prompt_fn {
                prompt(repo_name, &current_config, target_config)
                    .await
                    .unwrap_or(false)
            } else {
                // No prompt function provided, default to not updating
                false
            }
        }
        ConfigCommand::DryRun(_) => false, // Already handled above
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

    if errors.is_empty() {
        (
            Status::ConfigUpdated,
            format!("updated: {}", updates.join(", ")),
        )
    } else {
        (
            Status::ConfigError,
            format!("failed to update: {}", errors.join(", ")),
        )
    }
}

/// Validates if the provided email address is valid
#[allow(dead_code)]
pub fn is_valid_email(email: &str) -> bool {
    if email.is_empty() {
        return false;
    }

    // Basic email validation - contains @ and has content before and after
    let parts: Vec<&str> = email.split('@').collect();
    parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() && parts[1].contains('.')
}

/// Validates if the provided name is valid
#[allow(dead_code)]
pub fn is_valid_name(name: &str) -> bool {
    !name.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::parse_user_config;

    #[test]
    fn user_config_records_are_parsed_in_one_batch() {
        let config = parse_user_config(b"user.name\nMiko Meow\0user.email\nmiko@example.com\0")
            .expect("valid config records");

        assert_eq!(config.0.as_deref(), Some("Miko Meow"));
        assert_eq!(config.1.as_deref(), Some("miko@example.com"));
    }

    #[test]
    fn later_effective_values_win() {
        let config = parse_user_config(b"user.name\nGlobal\0user.name\nLocal\0")
            .expect("valid config records");

        assert_eq!(config.0.as_deref(), Some("Local"));
    }

    #[test]
    fn malformed_user_config_fails_closed() {
        assert!(parse_user_config(b"user.name-without-value\0").is_err());
    }
}
