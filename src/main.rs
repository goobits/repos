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
use commands::sync::handle_sync_command;
use commands::user::{handle_user_command, parse_user_command};
use git::UserArgs;

#[derive(Subcommand, Clone)]
enum Commands {
    /// Sync git repositories - push unpushed commits to remotes
    Sync {
        /// Automatically push branches with no upstream tracking
        #[arg(long)]
        force: bool,
    },
    /// Manage user configuration across repositories
    User {
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
#[command(version = "1.0")]
struct Cli {
    /// Automatically push branches with no upstream tracking (for sync)
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
        Some(Commands::Sync { force }) => {
            let force_push = *force || cli.force;
            handle_sync_command(force_push).await
        }
        Some(Commands::User {
            name,
            email,
            from_global,
            from_current,
            force,
            dry_run,
        }) => {
            let user_args = UserArgs {
                command: parse_user_command(
                    name.clone(),
                    email.clone(),
                    *from_global,
                    *from_current,
                    *force,
                    *dry_run,
                )?,
            };
            handle_user_command(user_args).await
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
