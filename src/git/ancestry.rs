//! Exact repository ancestry inspection.

use anyhow::{Context, Result};
use std::path::Path;

use super::runner::run_git;

/// Commits reachable only from `HEAD` and only from its upstream.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct AheadBehind {
    pub(crate) ahead: u32,
    pub(crate) behind: u32,
}

/// Counts both sides from one revision-graph snapshot.
pub(crate) async fn ahead_behind(path: &Path) -> Result<AheadBehind> {
    ahead_behind_between(path, "HEAD", "@{upstream}").await
}

/// Counts commits reachable only from two exact revisions.
pub(crate) async fn ahead_behind_between(
    path: &Path,
    head: &str,
    upstream: &str,
) -> Result<AheadBehind> {
    let range = format!("{head}...{upstream}");
    let (success, stdout, stderr) =
        run_git(path, &["rev-list", "--left-right", "--count", &range]).await?;
    if !success {
        anyhow::bail!(
            "{}",
            if stderr.is_empty() {
                "ahead/behind inspection failed"
            } else {
                &stderr
            }
        );
    }

    let mut fields = stdout.split_whitespace();
    let ahead = fields
        .next()
        .context("ahead/behind output omitted the ahead count")?
        .parse::<u32>()
        .with_context(|| format!("invalid ahead/behind output '{stdout}'"))?;
    let behind = fields
        .next()
        .context("ahead/behind output omitted the behind count")?
        .parse::<u32>()
        .with_context(|| format!("invalid ahead/behind output '{stdout}'"))?;
    if fields.next().is_some() {
        anyhow::bail!("invalid ahead/behind output '{stdout}'");
    }

    Ok(AheadBehind { ahead, behind })
}
