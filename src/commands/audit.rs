//! Security auditing command implementation
//!
//! This module handles the audit command which performs:
//! - Repository hygiene checking
//! - Secret scanning with TruffleHog
//! - Automated fixing of detected issues

use anyhow::Result;

use crate::audit::{scanner::run_truffle_scan, fixes::apply_fixes};
use crate::core::{set_terminal_title, set_terminal_title_and_flush};

/// Main handler for the audit command with fix capabilities
pub async fn handle_audit_command(
    install_tools: bool,
    verify: bool,
    json: bool,
    interactive: bool,
    fix_gitignore: bool,
    fix_large: bool,
    fix_secrets: bool,
    fix_all: bool,
    dry_run: bool,
    target_repos: Option<Vec<String>>,
) -> Result<()> {
    set_terminal_title("🚀 sync-repos audit");

    // Run TruffleHog secret scanning
    let (truffle_stats, hygiene_stats) = run_truffle_scan(
        install_tools,
        verify,
        json,
        target_repos.clone(),
    ).await?;

    // If any fix options are specified, apply them
    if interactive || fix_gitignore || fix_large || fix_secrets || fix_all {
        let fix_options = if fix_all {
            crate::audit::fixes::FixOptions::fix_all(dry_run, target_repos.clone())
        } else {
            crate::audit::fixes::FixOptions {
                interactive,
                fix_gitignore,
                fix_large,
                fix_secrets,
                untrack_files: false,
                dry_run,
                target_repos: target_repos.clone(),
            }
        };

        apply_fixes(&hygiene_stats, fix_options).await?;
    }

    set_terminal_title_and_flush("✅ sync-repos");

    // Exit with error code if secrets were found and verify flag is set
    if verify && truffle_stats.verified_secrets > 0 {
        println!("\n❌ Verified secrets found - exiting with error code 1");
        std::process::exit(1);
    }

    Ok(())
}