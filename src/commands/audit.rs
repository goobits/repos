//! Security auditing command implementation
//!
//! This module handles the audit command which performs:
//! - Repository hygiene checking
//! - Secret scanning with `TruffleHog`
//! - Automated fixing of detected issues

use anyhow::Result;

use crate::audit::{fixes::apply_fixes, scanner::run_truffle_scan};
use crate::core::{set_terminal_title, set_terminal_title_and_flush};

/// Main handler for the audit command with fix capabilities
#[allow(clippy::too_many_arguments)]
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
    set_terminal_title("🚀 repos audit");

    // Run TruffleHog secret scanning
    let (truffle_stats, hygiene_stats) =
        run_truffle_scan(install_tools, verify, json, target_repos.clone()).await?;

    if !truffle_stats.failed_repos.is_empty() || hygiene_stats.error_count() > 0 {
        anyhow::bail!(
            "audit incomplete: {} secret scans and {} hygiene scans failed",
            truffle_stats.failed_repos.len(),
            hygiene_stats.error_count()
        );
    }

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
                skip_confirm: false,
                target_repos: target_repos.clone(),
            }
        };

        let results = apply_fixes(&hygiene_stats, fix_options).await?;
        let failed_fixes = results
            .iter()
            .filter(|result| !result.errors.is_empty())
            .count();
        if failed_fixes > 0 {
            anyhow::bail!("{failed_fixes} repositories had failed audit fixes");
        }
    }

    set_terminal_title_and_flush("✅ repos");

    // Exit with error code if secrets were found and verify flag is set
    if verify && truffle_stats.verified_secrets > 0 {
        anyhow::bail!("verified secrets found");
    }

    Ok(())
}
