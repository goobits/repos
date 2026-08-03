//! Security auditing command implementation
//!
//! This module handles the audit command which performs:
//! - Repository hygiene checking
//! - Secret scanning with `TruffleHog`
//! - Automated fixing of detected issues

use anyhow::Result;

use crate::audit::{
    fixes::{apply_fixes, FixResult},
    hygiene::HygieneStatistics,
    scanner::{run_truffle_scan, TruffleStatistics},
};
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
    if !json {
        set_terminal_title("🚀 repos audit");
    }

    // Run TruffleHog secret scanning
    let (truffle_stats, hygiene_stats) =
        run_truffle_scan(install_tools, verify, json, target_repos.clone()).await?;

    if !truffle_stats.failed_repos.is_empty() || hygiene_stats.error_count() > 0 {
        if json {
            print_audit_json(&truffle_stats, &hygiene_stats, None, dry_run)?;
        }
        anyhow::bail!(
            "audit incomplete: {} secret scans and {} hygiene scans failed",
            truffle_stats.failed_repos.len(),
            hygiene_stats.error_count()
        );
    }

    // If any fix options are specified, apply them
    let mut fix_results = None;
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
        fix_results = Some(results);
        if failed_fixes > 0 {
            if json {
                print_audit_json(
                    &truffle_stats,
                    &hygiene_stats,
                    fix_results.as_deref(),
                    dry_run,
                )?;
            }
            anyhow::bail!("{failed_fixes} repositories had failed audit fixes");
        }
    }

    if json {
        print_audit_json(
            &truffle_stats,
            &hygiene_stats,
            fix_results.as_deref(),
            dry_run,
        )?;
    }

    if !json {
        set_terminal_title_and_flush("✅ repos");
    }

    // Exit with error code if secrets were found and verify flag is set
    if verify && truffle_stats.verified_secrets > 0 {
        anyhow::bail!("verified secrets found");
    }

    Ok(())
}

fn print_audit_json(
    truffle_stats: &TruffleStatistics,
    hygiene_stats: &HygieneStatistics,
    fix_results: Option<&[FixResult]>,
    dry_run: bool,
) -> Result<()> {
    let fixes = fix_results.map(|results| {
        let successful = results
            .iter()
            .filter(|result| result.errors.is_empty())
            .count();
        let failed = results.len().saturating_sub(successful);
        serde_json::json!({
            "dry_run": dry_run,
            "successful_repos": successful,
            "failed_repos": failed,
            "results": results,
        })
    });
    let report = serde_json::json!({
        "truffle": truffle_stats.to_json(),
        "hygiene": hygiene_stats.to_json(),
        "fixes": fixes,
        "message": truffle_stats.generate_summary(),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
