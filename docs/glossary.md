# Glossary

Quick reference for `repos` commands, flags, and concepts.

## Commands

**`repos status`** - Show fleet state: worktree changes, branch, and upstream state.

**`repos save`** - Stage tracked changes, commit, and push in one step.

**`repos sync`** - Fetch, rebase safe repositories, and report nested drift.

**`repos stage`** - Stage files matching a pattern.

**`repos unstage`** - Unstage files matching a pattern.

**`repos commit`** - Commit already staged changes.

**`repos push`** - Push unpushed commits.

**`repos pull`** - Granular Git-shaped pull command.

**`repos audit`** - Scan for secrets and hygiene issues.

**`repos publish`** - Publish detected packages.

**`repos doctor`** - Diagnose remotes, upstreams, dirty worktrees, conflicts, and nested drift.

**`repos nested`** - Manage nested repository drift.

**`repos config`** - Sync Git identity/config across repositories.

## Common Flags

**`--dry-run`** - Preview planned changes without mutating repositories.

**`--auto-upstream`** - Set upstream automatically for branches without tracking.

**`--include-untracked`** - Include untracked files in `repos save`.

**`--all`** - Include all non-ignored changes for `repos save`, or show all nested repos for `repos nested status`.

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

**Force push** - Rewriting remote history. Use `git push --force-with-lease` manually when history rewrite tools require it.

## Output Indicators

**🟢** - Success or clean state.

**🟡** - Warning, dirty state, or missing upstream.

**🟠** - Skipped or no-op.

**🔴** - Error or failed operation.
