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
mod package;
mod subrepo;
mod utils;

use commands::audit::handle_audit_command;
use commands::config::{handle_config_command, parse_config_command};
use commands::publish::handle_publish_command;
use commands::staging::{
    handle_commit_command, handle_stage_command, handle_staging_status_command,
    handle_unstage_command,
};
use commands::sync::handle_push_command;
use git::ConfigArgs;

#[derive(Subcommand, Clone)]
enum Commands {
    /// Push unpushed commits to remotes across all repositories
    Push {
        /// Automatically push branches with no upstream tracking
        #[arg(long)]
        force: bool,
        /// Show detailed progress for all repositories
        #[arg(long, short)]
        verbose: bool,
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
    /// Commit staged changes across all repositories
    Commit {
        /// Commit message
        message: String,
        /// Include repositories with no staged changes (create empty commits)
        #[arg(long)]
        include_empty: bool,
    },
    /// Publish packages to their registries (npm, cargo, PyPI)
    Publish {
        /// Specific repositories to publish (by name)
        repos: Vec<String>,
        /// Show what would be published without actually publishing
        #[arg(long)]
        dry_run: bool,
        /// Create and push git tags after successful publish (e.g., v1.2.3)
        #[arg(long)]
        tag: bool,
        /// Allow publishing with uncommitted changes (not recommended)
        #[arg(long)]
        allow_dirty: bool,
        /// Publish all repositories regardless of visibility
        #[arg(long, conflicts_with_all = ["public_only", "private_only"])]
        all: bool,
        /// Only publish public repositories (default behavior)
        #[arg(long, conflicts_with_all = ["all", "private_only"])]
        public_only: bool,
        /// Only publish private repositories
        #[arg(long, conflicts_with_all = ["all", "public_only"])]
        private_only: bool,
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
    /// Manage nested repository synchronization
    Subrepo {
        #[command(subcommand)]
        subcommand: SubrepoCommand,
    },
}

#[derive(Subcommand, Clone)]
enum SubrepoCommand {
    /// Validate subrepo setup and show all nested repos
    Validate,

    /// Show subrepo sync status (drift detection)
    Status {
        /// Show all subrepos, not just drifted ones
        #[arg(long)]
        all: bool,
    },

    /// Sync a subrepo to specific commit across all parents
    Sync {
        /// Subrepo name
        name: String,
        /// Target commit hash
        #[arg(long)]
        to: String,
        /// Stash uncommitted changes before syncing (safe, reversible)
        #[arg(long)]
        stash: bool,
        /// Force sync even with uncommitted changes (discards changes)
        #[arg(long)]
        force: bool,
    },

    /// Update a subrepo to latest commit across all parents
    Update {
        /// Subrepo name
        name: String,
        /// Force update even with uncommitted changes
        #[arg(long)]
        force: bool,
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

/// Handles subrepo subcommands
fn handle_subrepo_command(subcommand: SubrepoCommand) -> Result<()> {
    match subcommand {
        SubrepoCommand::Validate => {
            let report = subrepo::validation::validate_subrepos()?;
            subrepo::validation::display_report(&report);
            Ok(())
        }
        SubrepoCommand::Status { all } => {
            let statuses = subrepo::status::analyze_subrepos()?;
            subrepo::status::display_status(&statuses, all);
            Ok(())
        }
        SubrepoCommand::Sync { name, to, stash, force } => {
            subrepo::sync::sync_subrepo(&name, &to, stash, force)
        }
        SubrepoCommand::Update { name, force } => {
            subrepo::sync::update_subrepo(&name, force)
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Determine the operation mode and handle commands
    match &cli.command {
        Some(Commands::Push { force, verbose }) => {
            let force_push = *force || cli.force;
            handle_push_command(force_push, *verbose).await
        }
        Some(Commands::Stage { pattern }) => handle_stage_command(pattern.clone()).await,
        Some(Commands::Unstage { pattern }) => handle_unstage_command(pattern.clone()).await,
        Some(Commands::Status) => handle_staging_status_command().await,
        Some(Commands::Commit {
            message,
            include_empty,
        }) => handle_commit_command(message.clone(), *include_empty).await,
        Some(Commands::Publish { repos, dry_run, tag, allow_dirty, all, public_only, private_only }) => {
            handle_publish_command(repos.clone(), *dry_run, *tag, *allow_dirty, *all, *public_only, *private_only).await
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
        Some(Commands::Subrepo { subcommand }) => {
            handle_subrepo_command(subcommand.clone())
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
