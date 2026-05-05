//! repos: A tool for managing and synchronizing multiple git repositories
//!
//! This tool scans for git repositories and provides commands to:
//! - Push any unpushed commits to their upstream remotes
//! - Sync user configuration (name/email) across repositories
#![allow(clippy::large_enum_variant)]

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
use commands::doctor::handle_doctor_command;
use commands::publish::handle_publish_command;
use commands::save::handle_save_command;
use commands::staging::{
    handle_commit_command, handle_stage_command, handle_staging_status_command,
    handle_unstage_command,
};
use commands::sync::{handle_pull_command, handle_push_command};
use git::ConfigArgs;

#[derive(Subcommand, Clone)]
enum Commands {
    /// Stage tracked changes, commit, and push in one step
    Save {
        /// Commit message
        message: String,
        /// Include untracked files in the save
        #[arg(long, short = 'u', conflicts_with = "all")]
        include_untracked: bool,
        /// Stage all non-ignored changes, including untracked files
        #[arg(long, short = 'a', conflicts_with = "include_untracked")]
        all: bool,
        /// Set upstream automatically for branches without tracking
        #[arg(long)]
        auto_upstream: bool,
        /// Print the save plan without mutating repositories
        #[arg(long)]
        dry_run: bool,
    },
    /// Fetch, pull with rebase, and report nested drift
    Sync {
        /// Show detailed progress for all repositories
        #[arg(long, short)]
        verbose: bool,
        /// Show file changes in repos with uncommitted changes
        #[arg(long, short = 'c')]
        show_changes: bool,
        /// Skip nested repository drift check
        #[arg(long)]
        no_drift_check: bool,
        /// Number of concurrent operations (advanced)
        #[arg(long, short = 'j', conflicts_with = "sequential", hide = true)]
        jobs: Option<usize>,
        /// Run one operation at a time (advanced)
        #[arg(long, hide = true)]
        sequential: bool,
    },
    /// Push unpushed commits to remotes across all repositories
    Push {
        /// Set upstream automatically for branches without tracking
        #[arg(long)]
        auto_upstream: bool,
        /// Show detailed progress for all repositories
        #[arg(long, short)]
        verbose: bool,
        /// Show file changes in repos with uncommitted changes
        #[arg(long, short = 'c')]
        show_changes: bool,
        /// Skip nested repository drift check
        #[arg(long)]
        no_drift_check: bool,
        /// Number of concurrent operations (advanced)
        #[arg(long, short = 'j', conflicts_with = "sequential", hide = true)]
        jobs: Option<usize>,
        /// Run one operation at a time (advanced)
        #[arg(long, hide = true)]
        sequential: bool,
    },
    /// Pull changes from remotes across all repositories
    Pull {
        /// Use rebase instead of merge (git pull --rebase)
        #[arg(long)]
        rebase: bool,
        /// Show detailed progress for all repositories
        #[arg(long, short)]
        verbose: bool,
        /// Show file changes in repos with uncommitted changes
        #[arg(long, short = 'c')]
        show_changes: bool,
        /// Skip nested repository drift check
        #[arg(long)]
        no_drift_check: bool,
        /// Number of concurrent operations (advanced)
        #[arg(long, short = 'j', conflicts_with = "sequential", hide = true)]
        jobs: Option<usize>,
        /// Run one operation at a time (advanced)
        #[arg(long, hide = true)]
        sequential: bool,
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
        /// Apply without prompting
        #[arg(long)]
        yes: bool,
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
    /// Publish packages to their registries (npm, cargo, `PyPI`)
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
        /// Install required tools (`TruffleHog`) without prompting
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
    Nested {
        #[command(subcommand)]
        subcommand: NestedCommand,
    },
    /// Diagnose auth, remotes, nested state, and common blockers
    Doctor,
}

#[derive(Subcommand, Clone)]
enum NestedCommand {
    /// Validate nested repository setup and show all nested repos
    Validate,

    /// Show nested repository sync status (drift detection)
    Status {
        /// Show all nested repositories, not just drifted ones
        #[arg(long)]
        all: bool,
    },

    /// Sync a nested repository to specific commit across all parents
    Sync {
        /// Nested repository name
        name: String,
        /// Target commit hash
        #[arg(long)]
        to: String,
        /// Stash uncommitted changes before syncing (safe, reversible)
        #[arg(long)]
        stash: bool,
    },

    /// Update a nested repository to latest commit across all parents
    Update {
        /// Nested repository name
        name: String,
    },
}

#[derive(Parser)]
#[command(name = "repos")]
#[command(about = "Fleet-scale Git orchestration for humans")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

/// Handles nested repository subcommands.
fn handle_nested_command(subcommand: NestedCommand) -> Result<()> {
    match subcommand {
        NestedCommand::Validate => {
            let report = subrepo::validation::validate_subrepos()?;
            subrepo::validation::display_report(&report);
            Ok(())
        }
        NestedCommand::Status { all } => {
            let statuses = subrepo::status::analyze_subrepos()?;
            subrepo::status::display_status(&statuses, all);
            Ok(())
        }
        NestedCommand::Sync { name, to, stash } => {
            subrepo::sync::sync_subrepo(&name, &to, stash, false)
        }
        NestedCommand::Update { name } => subrepo::sync::update_subrepo(&name, false),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Determine the operation mode and handle commands
    match &cli.command {
        Some(Commands::Save {
            message,
            include_untracked,
            all,
            auto_upstream,
            dry_run,
        }) => {
            handle_save_command(
                message.clone(),
                *include_untracked,
                *all,
                *auto_upstream,
                *dry_run,
            )
            .await
        }
        Some(Commands::Sync {
            verbose,
            show_changes,
            no_drift_check,
            jobs,
            sequential,
        }) => {
            handle_pull_command(
                true,
                *verbose,
                *show_changes,
                *no_drift_check,
                *jobs,
                *sequential,
            )
            .await
        }
        Some(Commands::Push {
            auto_upstream,
            verbose,
            show_changes,
            no_drift_check,
            jobs,
            sequential,
        }) => {
            handle_push_command(
                *auto_upstream,
                *verbose,
                *show_changes,
                *no_drift_check,
                *jobs,
                *sequential,
            )
            .await
        }
        Some(Commands::Pull {
            rebase,
            verbose,
            show_changes,
            no_drift_check,
            jobs,
            sequential,
        }) => {
            handle_pull_command(
                *rebase,
                *verbose,
                *show_changes,
                *no_drift_check,
                *jobs,
                *sequential,
            )
            .await
        }
        Some(Commands::Stage { pattern }) => handle_stage_command(pattern.clone()).await,
        Some(Commands::Unstage { pattern }) => handle_unstage_command(pattern.clone()).await,
        Some(Commands::Status) => handle_staging_status_command().await,
        Some(Commands::Commit {
            message,
            include_empty,
        }) => handle_commit_command(message.clone(), *include_empty).await,
        Some(Commands::Publish {
            repos,
            dry_run,
            tag,
            allow_dirty,
            all,
            public_only,
            private_only,
        }) => {
            handle_publish_command(
                repos.clone(),
                *dry_run,
                *tag,
                *allow_dirty,
                *all,
                *public_only,
                *private_only,
            )
            .await
        }
        Some(Commands::Config {
            name,
            email,
            from_global,
            from_current,
            yes,
            dry_run,
        }) => {
            let config_args = ConfigArgs {
                command: parse_config_command(
                    name.clone(),
                    email.clone(),
                    *from_global,
                    *from_current,
                    *yes,
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
        Some(Commands::Nested { subcommand }) => handle_nested_command(subcommand.clone()),
        Some(Commands::Doctor) => handle_doctor_command().await,
        None => {
            // Default behavior - show help
            use clap::CommandFactory;
            let mut cmd = Cli::command();
            cmd.print_help()?;
            Ok(())
        }
    }
}
