# Glossary

Quick reference for `repos` commands, flags, and concepts.

## Commands

**`repos status`** - Show fleet state: worktree changes, branch, and upstream state.

**`repos save`** - Stage tracked changes, commit, and push in one step.

**`repos sync`** - Pull safe remote changes, push local commits, and report nested drift.

**`repos fetch`** - Refresh remote-tracking refs without changing local branches or worktrees.

**`repos stage`** - Stage files matching a pattern.

**`repos unstage`** - Unstage files matching a pattern.

**`repos commit`** - Commit already staged changes.

**`repos push`** - Push unpushed commits.

**`repos pull`** - Granular Git-shaped pull command.

**`repos audit`** - Scan for secrets and hygiene issues.

**`repos publish`** - Publish detected packages.

**`repos doctor`** - Diagnose remote access, upstreams, dirty worktrees, conflicts, and nested drift.

**`repos nested`** - Manage nested repository drift.

**`repos config`** - Sync Git identity/config across repositories.

## Common Flags

**`--dry-run`** - Preview planned changes without mutating repositories.

**`--auto-upstream`** - Set upstream automatically for branches without tracking.

**`--all`** - Include all non-ignored changes for `repos save`, or enumerate every discovered nested repository for `repos nested status`, including synced, unique, missing-origin, submodule, linked-worktree, and declared-but-uninitialized submodule entries.

**`--no-drift-check`** - Skip nested drift checks in `sync`, `push`, or `pull`.

**`--verbose` / `-v`** - Show detailed operation logs.

**`--repos <repo1,repo2>`** - Target specific repositories for audit.

## Nested Repository Terms

**Nested repository** - A Git repository inside another repository, with its own `.git` directory.

**Drift** - The same nested repository exists at different commits across parent repositories.

**Sync target** - The commit suggested for bringing drifted nested repositories back together.

## Git Terms

**Tracked change** - A modification or deletion to a file Git already tracks.

**Untracked file** - A file Git does not track yet. `repos save` does not include these by default.

**Upstream branch** - The remote branch a local branch tracks.

**Force push** - Rewriting remote history. Coordinate guarded updates for every affected branch and tag; updating only the current branch is incomplete.

## Output Indicators

**🟢** - Success or clean state.

**🟡** - Warning, dirty state, or missing upstream.

**🟠** - Skipped or no-op.

**🔴** - Error or failed operation.
