//! sync-repos: A tool for synchronizing multiple git repositories
//!
//! This tool scans for git repositories and provides commands to:
//! - Push any unpushed commits to their upstream remotes
//! - Sync user configuration (name/email) across repositories

use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;
mod core;
mod git;
mod audit;
mod utils;

use git::UserArgs;
use commands::sync::handle_sync_command;
use commands::audit::handle_audit_command;
use commands::user::{handle_user_command, parse_user_command};

#[derive(Subcommand, Clone)]
enum Commands {
    /// Sync git repositories (default behavior)
    #[command(hide = true)]
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
        /// Automatically install TruffleHog without prompting
        #[arg(long)]
        auto_install: bool,
        /// Verify discovered secrets are active and fail on findings
        #[arg(long)]
        verify: bool,
        /// Output results in JSON format
        #[arg(long)]
        json: bool,
        /// Interactive fix mode - prompts for each type of fix
        #[arg(long)]
        fix: bool,
        /// Fix .gitignore violations by adding entries
        #[arg(long)]
        fix_gitignore: bool,
        /// Remove large files from Git history (requires git filter-repo)
        #[arg(long)]
        fix_large: bool,
        /// Remove secrets from Git history
        #[arg(long)]
        fix_secrets: bool,
        /// Apply safe fixes automatically (only .gitignore additions)
        #[arg(long)]
        auto_fix: bool,
        /// Preview changes without applying them
        #[arg(long)]
        dry_run: bool,
        /// Only fix specific repositories (comma-separated)
        #[arg(long, value_delimiter = ',')]
        repos: Option<Vec<String>>,
    },
}

#[derive(Parser)]
#[command(name = "sync-repos")]
#[command(about = "A tool for synchronizing multiple git repositories")]
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
            auto_install,
            verify,
            json,
            fix,
            fix_gitignore,
            fix_large,
            fix_secrets,
            auto_fix,
            dry_run,
            repos,
        }) => handle_audit_command(
            *auto_install,
            *verify,
            *json,
            *fix,
            *fix_gitignore,
            *fix_large,
            *fix_secrets,
            *auto_fix,
            *dry_run,
            repos.clone(),
        ).await,
        None => {
            // Default behavior - run sync command
            handle_sync_command(cli.force).await
        }
    }
}
