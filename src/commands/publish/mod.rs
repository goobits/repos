//! Repository publish command implementation

mod planner;
mod executor;

use anyhow::Result;
use crate::core::{init_command, set_terminal_title, set_terminal_title_and_flush, NO_REPOS_MESSAGE};
use planner::{plan_publish, PlannerOptions};
use executor::execute_publish;

const SCANNING_MESSAGE: &str = "🔍 Scanning for packages...";

/// Handles the repository publish command
pub async fn handle_publish_command(
    target_repos: Vec<String>,
    dry_run: bool,
    tag: bool,
    allow_dirty: bool,
    all: bool,
    _public_only: bool, // unused but kept for signature compatibility if needed
    private_only: bool,
) -> Result<()> {
    set_terminal_title("📦 repos");

    let (start_time, repos) = init_command(SCANNING_MESSAGE);

    if repos.is_empty() {
        println!("\r{NO_REPOS_MESSAGE}");
        set_terminal_title_and_flush("✅ repos");
        return Ok(());
    }

    // Plan the publish operation
    let options = PlannerOptions {
        target_repos: target_repos.clone(),
        all,
        private_only,
        allow_dirty,
        dry_run,
    };

    let plan = plan_publish(repos, options).await;

    // Show filtering feedback
    if plan.skipped_count > 0 {
        let visibility_type = if private_only { "private" } else { "public" };
        let mut skip_msg = format!("\r📦 Filtered to {visibility_type} repos only ({}) skipped)", plan.skipped_count);
        if plan.unknown_count > 0 {
            skip_msg.push_str(&format!(" [{} unknown visibility treated as private]", plan.unknown_count));
        }
        println!("{skip_msg}\n");
    }

    // Check for dirty repos
    if !plan.dirty_repos.is_empty() && !allow_dirty && !dry_run {
        println!(
            "\r❌ Cannot publish: {} {} uncommitted changes\n",
            plan.dirty_repos.len(),
            if plan.dirty_repos.len() == 1 { "repository has" } else { "repositories have" }
        );
        println!("Repositories with uncommitted changes:");
        for repo in &plan.dirty_repos {
            println!("  • {repo}");
        }
        println!("\nCommit your changes first, or use --allow-dirty to publish anyway (not recommended).\n");
        set_terminal_title_and_flush("✅ repos");
        return Ok(());
    }

    if plan.packages.is_empty() {
        if target_repos.is_empty() {
            println!("\r📦 No packages found in any repository\n");
        } else {
            println!("\r📦 No packages found matching: {}\n", target_repos.join(", "));
        }
        set_terminal_title_and_flush("✅ repos");
        return Ok(());
    }

    // Dry Run
    if dry_run {
        println!("\r📦 Found {} packages (dry-run mode)\n", plan.packages.len());

        for pkg in &plan.packages {
            if let Some(info) = pkg.manager.get_info(&pkg.path).await {
                println!("  {} {:<30} ({:<7}) v{}", pkg.manager.icon(), pkg.name, pkg.manager.name(), info.version);
            } else {
                println!("  {} {:<30} ({:<7}) version unknown", pkg.manager.icon(), pkg.name, pkg.manager.name());
            }
        }
        println!("\nWould publish {} packages (dry-run - nothing published)\n", plan.packages.len());
        set_terminal_title_and_flush("✅ repos");
        return Ok(());
    }

    // Execute
    let total_packages = plan.packages.len();
    let package_word = if total_packages == 1 { "package" } else { "packages" };
    print!("\r📦 Publishing {total_packages} {package_word}                    \n");
    println!();

    execute_publish(plan.packages, tag, start_time).await?;

    set_terminal_title_and_flush("✅ repos");
    Ok(())
}