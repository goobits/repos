//! NUL-safe parsing of Git porcelain-v2 worktree state.

use anyhow::{Context, Result};
use std::path::Path;

use super::runner::run_git_raw;

#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) enum HeadState {
    Branch(String),
    Detached,
    Unborn,
    #[default]
    Unknown,
}

const STATUS_ARGS: &[&str] = &[
    "status",
    "--porcelain=v2",
    "-z",
    "--untracked-files=normal",
    "--ignore-submodules=dirty",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorktreeEntryKind {
    Ordinary,
    RenameOrCopy,
    Unmerged,
    Untracked,
    Ignored,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct WorktreeState {
    entries: Vec<WorktreeEntryKind>,
    head: HeadState,
    upstream: Option<String>,
    ahead_behind: Option<super::ancestry::AheadBehind>,
}

impl WorktreeState {
    pub(crate) fn is_dirty(&self) -> bool {
        self.entries
            .iter()
            .any(|kind| *kind != WorktreeEntryKind::Ignored)
    }

    pub(crate) fn has_conflicts(&self) -> bool {
        self.entries.contains(&WorktreeEntryKind::Unmerged)
    }

    pub(crate) fn has_tracked_changes(&self) -> bool {
        self.entries.iter().any(|kind| {
            matches!(
                kind,
                WorktreeEntryKind::Ordinary
                    | WorktreeEntryKind::RenameOrCopy
                    | WorktreeEntryKind::Unmerged
            )
        })
    }

    pub(crate) fn has_untracked_changes(&self) -> bool {
        self.entries.contains(&WorktreeEntryKind::Untracked)
    }

    pub(crate) fn head(&self) -> &HeadState {
        &self.head
    }

    pub(crate) fn upstream(&self) -> Option<&str> {
        self.upstream.as_deref()
    }

    pub(crate) fn ahead_behind(&self) -> Option<super::ancestry::AheadBehind> {
        self.ahead_behind
    }
}

pub(crate) async fn inspect_worktree(path: &Path) -> Result<WorktreeState> {
    let output = run_git_raw(path, STATUS_ARGS).await?;
    if !output.success() {
        let stderr = output.stderr_text();
        anyhow::bail!(
            "{}",
            if stderr.is_empty() {
                "worktree inspection failed"
            } else {
                &stderr
            }
        );
    }
    parse_porcelain_v2_z(&output.stdout)
}

pub(crate) async fn inspect_repository_state(path: &Path) -> Result<WorktreeState> {
    let mut args = STATUS_ARGS.to_vec();
    args.push("--branch");
    let output = run_git_raw(path, &args).await?;
    if !output.success() {
        let stderr = output.stderr_text();
        anyhow::bail!(
            "{}",
            if stderr.is_empty() {
                "repository state inspection failed"
            } else {
                &stderr
            }
        );
    }
    parse_porcelain_v2_z(&output.stdout)
}

pub(crate) async fn inspect_refreshed_repository_state(path: &Path) -> Result<WorktreeState> {
    // Refresh is an optimization hint; porcelain status remains authoritative
    // for unborn repositories and unusual index states if refresh cannot run.
    let _ = run_git_raw(path, &["update-index", "--refresh"]).await;
    inspect_repository_state(path).await
}

fn parse_porcelain_v2_z(output: &[u8]) -> Result<WorktreeState> {
    let mut records = output.split(|byte| *byte == 0).peekable();
    let mut state = WorktreeState::default();
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        let kind = match record[0] {
            b'1' => WorktreeEntryKind::Ordinary,
            b'2' => {
                let original_path = records
                    .next()
                    .context("rename/copy record omitted its original path")?;
                if original_path.is_empty() {
                    anyhow::bail!("rename/copy record has an empty original path");
                }
                WorktreeEntryKind::RenameOrCopy
            }
            b'u' => WorktreeEntryKind::Unmerged,
            b'?' => WorktreeEntryKind::Untracked,
            b'!' => WorktreeEntryKind::Ignored,
            b'#' => {
                parse_header(record, &mut state)?;
                continue;
            }
            byte => anyhow::bail!("unknown porcelain-v2 record type 0x{byte:02x}"),
        };
        state.entries.push(kind);
    }
    Ok(state)
}

fn parse_header(record: &[u8], state: &mut WorktreeState) -> Result<()> {
    let header = std::str::from_utf8(record).context("branch header is not valid UTF-8")?;
    if let Some(head) = header.strip_prefix("# branch.head ") {
        state.head = match head {
            "(detached)" => HeadState::Detached,
            "(initial)" => HeadState::Unborn,
            branch => HeadState::Branch(branch.to_string()),
        };
    } else if let Some(upstream) = header.strip_prefix("# branch.upstream ") {
        state.upstream = Some(upstream.to_string());
    } else if let Some(counts) = header.strip_prefix("# branch.ab ") {
        let (ahead, behind) = counts
            .split_once(' ')
            .context("branch.ab omitted one side")?;
        let ahead = ahead
            .strip_prefix('+')
            .context("branch.ab ahead count omitted '+'")?
            .parse()
            .context("invalid branch.ab ahead count")?;
        let behind = behind
            .strip_prefix('-')
            .context("branch.ab behind count omitted '-'")?
            .parse()
            .context("invalid branch.ab behind count")?;
        state.ahead_behind = Some(super::ancestry::AheadBehind { ahead, behind });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_paths_with_newlines_and_rename_pairs_without_line_splitting() {
        let output = b"1 .M N... 100644 100644 100644 abc abc path\nname\0\
                       2 R. N... 100644 100644 100644 abc def R100 new name\0old\nname\0\
                       ? untracked\nname\0";
        let state = parse_porcelain_v2_z(output).unwrap();
        assert!(state.is_dirty());
        assert!(!state.has_conflicts());
        assert_eq!(state.entries.len(), 3);
    }

    #[test]
    fn parses_branch_upstream_and_both_ancestry_counts() {
        let output = b"# branch.oid abc\0# branch.head main\0\
                       # branch.upstream origin/main\0# branch.ab +12 -3\0";
        let state = parse_porcelain_v2_z(output).unwrap();
        assert_eq!(state.head(), &HeadState::Branch("main".to_string()));
        assert_eq!(state.upstream(), Some("origin/main"));
        assert_eq!(
            state.ahead_behind(),
            Some(super::super::ancestry::AheadBehind {
                ahead: 12,
                behind: 3
            })
        );
    }

    #[test]
    fn detects_every_unmerged_record_as_a_conflict() {
        let output = b"u DD N... 100644 100644 100644 100644 a b c d conflict\0";
        assert!(parse_porcelain_v2_z(output).unwrap().has_conflicts());
    }

    #[test]
    fn rejects_truncated_rename_records() {
        let error =
            parse_porcelain_v2_z(b"2 R. N... 100644 100644 100644 abc def R100 new\0").unwrap_err();
        assert!(error.to_string().contains("original path"));
    }
}
