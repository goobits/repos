//! repos: A tool for managing and synchronizing multiple git repositories
//!
//! This tool scans for git repositories and provides commands to:
//! - Push any unpushed commits to their upstream remotes
//! - Sync user configuration (name/email) across repositories

use anyhow::Result;
use clap::{Parser, Subcommand};

mod audit;
mod commands;
mod core;
mod git;
mod utils;

use commands::audit::handle_audit_command;
use commands::staging::{handle_stage_command, handle_unstage_command, handle_staging_status_command};
use commands::sync::handle_push_command;
use commands::config::{handle_config_command, parse_config_command};
use git::ConfigArgs;

#[derive(Subcommand, Clone)]
enum Commands {
    /// Push unpushed commits to remotes across all repositories
    Push {
        /// Automatically push branches with no upstream tracking
        #[arg(long)]
        force: bool,
    },
    /// Manage git configuration across repositories
    Config {
        /// User name to set across all repositories
        #[arg(long)]
        name: Option<String>,
        /// User email to set across all repositories
        #[arg(long)]
        email: Option<String>,
        /// Use global git config as source
        #[arg(long, conflicts_with_all = ["name", "email", "from_current"])]
        from_global: bool,
        /// Use current repository's config as source
        #[arg(long, conflicts_with_all = ["name", "email", "from_global"])]
        from_current: bool,
        /// Force overwrite all configs without prompting
        #[arg(long)]
        force: bool,
        /// Show what would be changed without making changes
        #[arg(long)]
        dry_run: bool,
    },
    /// Stage files matching pattern across all repositories
    Stage {
        /// Pattern to match files (e.g., "*.md", "README.md")
        pattern: String,
    },
    /// Unstage files matching pattern across all repositories
    Unstage {
        /// Pattern to match files (e.g., "*.md", "README.md", "*")
        pattern: String,
    },
    /// Show staging status across all repositories
    Status,
    /// Audit repositories for security vulnerabilities and secrets
    Audit {
        /// Install required tools (TruffleHog) without prompting
        #[arg(long)]
        install_tools: bool,
        /// Verify discovered secrets are active and fail on findings
        #[arg(long)]
        verify: bool,
        /// Output results in JSON format
        #[arg(long)]
        json: bool,
        /// Interactive mode - choose fixes interactively
        #[arg(long)]
        interactive: bool,
        /// Fix .gitignore violations by adding entries
        #[arg(long)]
        fix_gitignore: bool,
        /// Remove large files from Git history (requires git filter-repo)
        #[arg(long)]
        fix_large: bool,
        /// Remove secrets from Git history
        #[arg(long)]
        fix_secrets: bool,
        /// Apply all available fixes automatically
        #[arg(long)]
        fix_all: bool,
        /// Preview changes without applying them
        #[arg(long)]
        dry_run: bool,
        /// Only fix specific repositories (comma-separated)
        #[arg(long, value_delimiter = ',')]
        repos: Option<Vec<String>>,
    },
}

#[derive(Parser)]
#[command(name = "repos")]
#[command(about = "A tool for managing and synchronizing multiple git repositories")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Automatically push branches with no upstream tracking (for push)
    #[arg(long, global = true)]
    force: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Determine the operation mode and handle commands
    match &cli.command {
        Some(Commands::Push { force }) => {
            let force_push = *force || cli.force;
            handle_push_command(force_push).await
        }
        Some(Commands::Stage { pattern }) => {
            handle_stage_command(pattern.clone()).await
        }
        Some(Commands::Unstage { pattern }) => {
            handle_unstage_command(pattern.clone()).await
        }
        Some(Commands::Status) => {
            handle_staging_status_command().await
        }
        Some(Commands::Config {
            name,
            email,
            from_global,
            from_current,
            force,
            dry_run,
        }) => {
            let config_args = ConfigArgs {
                command: parse_config_command(
                    name.clone(),
                    email.clone(),
                    *from_global,
                    *from_current,
                    *force,
                    *dry_run,
                )?,
            };
            handle_config_command(config_args).await
        }
        Some(Commands::Audit {
            install_tools,
            verify,
            json,
            interactive,
            fix_gitignore,
            fix_large,
            fix_secrets,
            fix_all,
            dry_run,
            repos,
        }) => {
            handle_audit_command(
                *install_tools,
                *verify,
                *json,
                *interactive,
                *fix_gitignore,
                *fix_large,
                *fix_secrets,
                *fix_all,
                *dry_run,
                repos.clone(),
            )
            .await
        }
        None => {
            // Default behavior - show help
            use clap::CommandFactory;
            let mut cmd = Cli::command();
            cmd.print_help()?;
            Ok(())
        }
    }
}
