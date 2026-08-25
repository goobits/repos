//! NUL-safe parsing of Git porcelain-v2 worktree state.

use anyhow::{Context, Result};
use std::path::Path;

use super::runner::run_git_raw;

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

fn parse_porcelain_v2_z(output: &[u8]) -> Result<WorktreeState> {
    let mut records = output.split(|byte| *byte == 0).peekable();
    let mut entries = Vec::new();
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
            b'#' => continue,
            byte => anyhow::bail!("unknown porcelain-v2 record type 0x{byte:02x}"),
        };
        entries.push(kind);
    }
    Ok(WorktreeState { entries })
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
