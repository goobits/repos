//! User configuration synchronization command implementation
//!
//! This module handles syncing git user.name and user.email across repositories
//! with interactive conflict resolution and validation.

use anyhow::Result;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::core::{
    create_processing_context, init_command, set_terminal_title, set_terminal_title_and_flush,
    ProcessingContext, CONFIG_SYNCING_MESSAGE, NO_REPOS_MESSAGE,
};
use crate::git::{
    check_repo_config, get_current_user_config, get_global_user_config, validate_user_config,
    ConfigSource, UserArgs, UserCommand, UserConfig,
};

const SCANNING_MESSAGE: &str = "🔍 Scanning for git repositories...";

/// Shows interactive prompt for config selection when no arguments provided
async fn show_config_selection_prompt() -> Result<UserArgs> {
    println!("\n📋 Git User Configuration Options\n");

    // Get global config
    let (global_name, global_email) = get_global_user_config().await;

    // Get current directory config
    let current_dir = std::env::current_dir()?;
    let (current_name, current_email) = get_current_user_config(&current_dir).await;

    println!("1) Global config (~/.gitconfig)");
    if let Some(name) = &global_name {
        println!("   Name:  {}", name);
    } else {
        println!("   Name:  <not set>");
    }
    if let Some(email) = &global_email {
        println!("   Email: {}", email);
    } else {
        println!("   Email: <not set>");
    }

    println!("\n2) Current directory config");
    if let Some(name) = &current_name {
        println!("   Name:  {}", name);
    } else {
        println!("   Name:  <not set>");
    }
    if let Some(email) = &current_email {
        println!("   Email: {}", email);
    } else {
        println!("   Email: <not set>");
    }

    println!("\n3) Enter custom values");
    println!("4) Cancel\n");

    print!("Select option [1-4]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let choice = input.trim();

    let config_source = match choice {
        "1" => {
            if global_name.is_none() && global_email.is_none() {
                println!("\n❌ No global config found. Use 'git config --global' to set values first.");
                std::process::exit(1);
            }
            println!("\n✅ Using global config to sync all repositories");
            ConfigSource::Global
        }
        "2" => {
            if current_name.is_none() && current_email.is_none() {
                println!("\n❌ No config found in current directory.");
                std::process::exit(1);
            }
            println!("\n✅ Using current directory config to sync all repositories");
            ConfigSource::Current(current_dir)
        }
        "3" => {
            print!("\nEnter name (or press Enter to skip): ");
            io::stdout().flush()?;
            let mut name_input = String::new();
            io::stdin().read_line(&mut name_input)?;
            let name = if name_input.trim().is_empty() {
                None
            } else {
                Some(name_input.trim().to_string())
            };

            print!("Enter email (or press Enter to skip): ");
            io::stdout().flush()?;
            let mut email_input = String::new();
            io::stdin().read_line(&mut email_input)?;
            let email = if email_input.trim().is_empty() {
                None
            } else {
                Some(email_input.trim().to_string())
            };

            if name.is_none() && email.is_none() {
                println!("\n❌ No values provided");
                std::process::exit(1);
            }

            let config = UserConfig::new(name, email);
            validate_user_config(&config)?;
            println!("\n✅ Using custom config to sync all repositories");
            ConfigSource::Explicit(config)
        }
        "4" | _ => {
            println!("\nCancelled");
            std::process::exit(0);
        }
    };

    Ok(UserArgs {
        command: UserCommand::Interactive(config_source),
    })
}

/// Parses user command arguments into a UserCommand
pub fn parse_user_command(
    name: Option<String>,
    email: Option<String>,
    from_global: bool,
    from_current: bool,
    force: bool,
    dry_run: bool,
) -> Result<UserCommand> {
    let config_source = if from_global {
        ConfigSource::Global
    } else if from_current {
        ConfigSource::Current(std::env::current_dir()?)
    } else if name.is_some() || email.is_some() {
        let config = UserConfig::new(name, email);
        validate_user_config(&config)?;
        ConfigSource::Explicit(config)
    } else {
        // No arguments provided - show interactive selection
        ConfigSource::Interactive
    };

    let command = if dry_run {
        UserCommand::DryRun(config_source)
    } else if force {
        UserCommand::Force(config_source)
    } else {
        UserCommand::Interactive(config_source)
    };

    Ok(command)
}

/// Resolves config source to actual UserConfig values
pub async fn resolve_config_source(
    source: &ConfigSource,
    _repos: &[(String, PathBuf)],
) -> Result<UserConfig> {
    match source {
        ConfigSource::Explicit(config) => Ok(config.clone()),
        ConfigSource::Global => {
            let (name, email) = get_global_user_config().await;
            Ok(UserConfig::new(name, email))
        }
        ConfigSource::Current(path) => {
            let (name, email) = get_current_user_config(path).await;
            Ok(UserConfig::new(name, email))
        }
        ConfigSource::Interactive => {
            // This should never be reached as Interactive is resolved earlier
            Err(anyhow::anyhow!("Interactive config source should be resolved before this point"))
        }
    }
}

/// Prompts user for individual repository config conflict resolution
pub async fn prompt_for_config_resolution(
    repo_name: &str,
    current: &UserConfig,
    target: &UserConfig,
) -> Result<bool> {
    println!("\n🔄 Config conflict in repository: {}", repo_name);

    if let (Some(current_name), Some(target_name)) = (&current.name, &target.name) {
        if current_name != target_name {
            println!("   Name:  {} → {}", current_name, target_name);
        }
    }

    if let (Some(current_email), Some(target_email)) = (&current.email, &target.email) {
        if current_email != target_email {
            println!("   Email: {} → {}", current_email, target_email);
        }
    }

    print!("Update config? [y/N]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    Ok(input.trim().to_lowercase().starts_with('y'))
}

/// Handles the user config command
pub async fn handle_user_command(args: UserArgs) -> Result<()> {
    set_terminal_title("🚀 sync-repos user");

    // Handle interactive config selection first if needed
    let resolved_args = if let UserCommand::Interactive(ConfigSource::Interactive) = &args.command {
        show_config_selection_prompt().await?
    } else {
        args
    };

    let (start_time, repos) = init_command(SCANNING_MESSAGE);

    if repos.is_empty() {
        println!("\r{}", NO_REPOS_MESSAGE);
        set_terminal_title_and_flush("✅ sync-repos");
        return Ok(());
    }

    // Determine target config based on source
    let target_config = match &resolved_args.command {
        UserCommand::Interactive(source)
        | UserCommand::Force(source)
        | UserCommand::DryRun(source) => resolve_config_source(source, &repos).await?,
    };

    if target_config.is_empty() {
        println!("\r❌ No user configuration found to sync");
        set_terminal_title_and_flush("✅ sync-repos");
        return Ok(());
    }

    let total_repos = repos.len();
    let repo_word = if total_repos == 1 {
        "repository"
    } else {
        "repositories"
    };
    let mode_text = match resolved_args.command {
        UserCommand::DryRun(_) => "(dry run)",
        UserCommand::Force(_) => "(force)",
        _ => "",
    };
    print!(
        "\r🚀 Syncing user config for {} {} {}                    \n",
        total_repos, repo_word, mode_text
    );
    println!();

    // Display target config
    if let Some(name) = &target_config.name {
        println!("📝 Target name:  {}", name);
    }
    if let Some(email) = &target_config.email {
        println!("📧 Target email: {}", email);
    }
    println!();

    // Create processing context
    let context = match create_processing_context(repos, start_time) {
        Ok(context) => context,
        Err(e) => {
            set_terminal_title_and_flush("✅ sync-repos");
            return Err(e);
        }
    };

    // Process all repositories concurrently for config sync
    process_config_repositories(context, resolved_args.command, target_config).await;

    set_terminal_title_and_flush("✅ sync-repos");
    Ok(())
}

/// Processes all repositories concurrently for config synchronization
async fn process_config_repositories(
    context: ProcessingContext,
    command: UserCommand,
    target_config: UserConfig,
) {
    use crate::core::{acquire_semaphore_permit, acquire_stats_lock, create_progress_bar};
    use futures::stream::{FuturesUnordered, StreamExt};
    use indicatif::{ProgressBar, ProgressStyle};

    let mut futures = FuturesUnordered::new();

    // First, create all repository progress bars
    let mut repo_progress_bars = Vec::new();
    for (repo_name, _) in &context.repositories {
        let progress_bar = create_progress_bar(&context.multi_progress, &context.progress_style, repo_name);
        progress_bar.set_message(CONFIG_SYNCING_MESSAGE);
        repo_progress_bars.push(progress_bar);
    }

    // Add a blank line before the footer
    let separator_pb = context.multi_progress.add(ProgressBar::new(0));
    separator_pb.set_style(ProgressStyle::default_bar().template(" ").unwrap());
    separator_pb.finish();

    // Create the footer progress bar
    let footer_pb = context.multi_progress.add(ProgressBar::new(0));
    let footer_style = ProgressStyle::default_bar()
        .template("{wide_msg}")
        .expect("Failed to create footer progress style");
    footer_pb.set_style(footer_style);

    // Initial footer display
    let initial_stats = crate::core::SyncStatistics::new();
    let initial_summary = initial_stats.generate_summary(context.total_repos, context.start_time.elapsed());
    footer_pb.set_message(initial_summary);

    // Add another blank line after the footer
    let separator_pb2 = context.multi_progress.add(ProgressBar::new(0));
    separator_pb2.set_style(ProgressStyle::default_bar().template(" ").unwrap());
    separator_pb2.finish();

    // Extract values we need in the async closures before moving context.repositories
    let max_name_length = context.max_name_length;
    let start_time = context.start_time;
    let total_repos = context.total_repos;

    for ((repo_name, repo_path), progress_bar) in context.repositories.into_iter().zip(repo_progress_bars) {
        let stats_clone = std::sync::Arc::clone(&context.statistics);
        let semaphore_clone = std::sync::Arc::clone(&context.semaphore);
        let footer_clone = footer_pb.clone();
        let command_clone = command.clone();
        let target_config_clone = target_config.clone();

        let future = async move {
            let _permit = acquire_semaphore_permit(&semaphore_clone).await;

            let (status, message) =
                check_repo_config(&repo_path, &repo_name, &target_config_clone, &command_clone)
                    .await;

            progress_bar.set_prefix(format!(
                "{} {:width$}",
                status.symbol(),
                repo_name,
                width = max_name_length
            ));
            progress_bar.set_message(format!("{:<12}   {}", status.text(), message));
            progress_bar.finish();

            // Update statistics
            let mut stats_guard = acquire_stats_lock(&stats_clone);
            let repo_path_str = repo_path.to_string_lossy();
            stats_guard.update(&repo_name, &repo_path_str, &status, &message, false);

            // Update the footer summary after each repository completes
            let duration = start_time.elapsed();
            let summary = stats_guard.generate_summary(total_repos, duration);
            footer_clone.set_message(summary);
        };

        futures.push(future);
    }

    // Wait for all repository operations to complete
    while futures.next().await.is_some() {}

    // Finish the footer progress bar
    footer_pb.finish();

    // Print the final detailed summary if there are any issues to report
    let final_stats = acquire_stats_lock(&context.statistics);
    let detailed_summary = final_stats.generate_detailed_summary();
    if !detailed_summary.is_empty() {
        println!("\n{}", "━".repeat(70));
        println!("{}", detailed_summary);
        println!("{}", "━".repeat(70));
    }

    // Add final spacing
    println!();
}
