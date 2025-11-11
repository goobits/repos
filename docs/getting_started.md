# Getting Started

Batch Git operations across multiple repositories. Instead of manually visiting each repo to run the same git commands, run one `repos` command across all repositories simultaneously.

## Installation

```bash
./install.sh  # See installation.md for alternatives
```

## Quick Tour

Run `repos` in a directory to discover and operate on all Git repositories:

```bash
repos                           # Discover repos
```

Output:
```
📦 Discovered 5 repositories
  my-app
  my-lib
  frontend
  backend
  docs
```

### Push All Repos

```bash
repos push                      # Push + drift check
```

Output:
```
✅ 3 repos pushed  ⚠️  2 already up-to-date

🔴 SUBREPO DRIFT (1)
auth: 2 instances at different commits
  → 105ce4e  beheremeow-app  ✅ clean  ⬆️ LATEST
    2f13c23  quest-keeper    ✅ clean  (outdated)
    Sync: repos subrepo sync auth --to 105ce4e
```

**Integrated Health Check**: `repos push` automatically checks for subrepo drift, showing you all repository health issues in one command.

### Stage Files Across Repos

```bash
repos stage "*.md"              # Stage markdown files
repos stage "README.md"         # Stage specific file
```

### Commit Changes

```bash
repos commit "Update docs"      # Commit staged changes
```

Output:
```
✅ 3 repos committed  ⚠️  2 skipped (no staged changes)
```

### Sync Git Config

```bash
repos config --from-global      # Copy from global config
repos config --name "Alice" --email "alice@example.com"  # Set directly
```

## Common Workflows

### Bulk Updates

Stage, commit, and push across all repos:

```bash
repos stage "*"                 # Stage all changes
repos commit "Bulk update"      # Commit everything
repos push                      # Push + drift check
```

### Release Workflow

Publish packages with git tags:

```bash
repos publish --dry-run         # Preview first
repos publish --tag             # Publish + create tags (v1.2.3)
```

**Note:** Publishing requires authentication. See [credentials setup](guides/credentials_setup.md) to configure npm, Cargo, or PyPI credentials.

### Config Sync

Set name/email across all repos:

```bash
repos config --from-global --force    # No prompts
```

## Frequently Asked Questions

### When should I use `repos` instead of manual git commands?

Use `repos` when you need to perform the same operation across multiple repositories. Instead of running `cd repo1 && git push && cd ../repo2 && git push...`, run `repos push` once.

### Does `repos` work with git submodules?

`repos` treats submodules as separate repositories. The `subrepo` commands are for nested repos (independent `.git` directories), not git submodules. Use `git submodule` commands for submodule management.

### Can I use `repos` in a monorepo?

Yes. `repos` works with any directory structure containing multiple git repositories. It discovers all repos recursively and operates on them concurrently.

### Does `repos` have a pull command?

Not yet. Use standard `git pull` directly in each repository. `repos` currently focuses on push operations, staging, commits, and publishing.

### What's the difference between `--force` and `--stash` in subrepo commands?

- **`--stash`**: Safely stashes uncommitted changes before syncing. Changes can be recovered with `git stash pop`
- **`--force`**: Permanently discards uncommitted changes. Use only when you're certain you don't need them

### How do I target specific repositories instead of all of them?

Most commands accept repository names as arguments:
```bash
repos publish my-app my-lib        # Only these repos
repos audit --repos my-app,my-lib  # Comma-separated for audit
```

### Why does publishing skip some of my repos?

By default, `repos publish` only publishes public repositories. Use `--all` to include private repos, or `--private-only` for only private repos.

### Is it safe to use `repos audit --fix-all`?

Only the `--fix-gitignore` operation is completely safe (just adds patterns to `.gitignore`). The `--fix-large` and `--fix-secrets` flags rewrite git history, which requires force-pushing and impacts all collaborators. Always run `--dry-run` first.

### How do I undo a history rewrite from `repos audit --fix-secrets`?

The tool creates backup refs before rewriting:
```bash
git reset --hard refs/original/pre-fix-backup-secrets-<timestamp>
```

### Do I need to install TruffleHog manually?

No. Run `repos audit --install-tools` and it will auto-install TruffleHog for you.

### Why is `repos audit --verify` slower than regular audit?

Verification mode tests whether detected secrets are currently active by making API calls to verify them. This is thorough but slower. Use in CI/CD to fail builds on active secrets.

### What does the arrow → mean in subrepo output?

The arrow points to the commit you should sync to. It marks the latest clean commit (no uncommitted changes), which is the safest sync target. Commits without the arrow are either outdated or have uncommitted changes.

### Can I use `repos` with private package registries?

Yes. Configure your private registry credentials the same way you would for the package manager:
- **npm**: `.npmrc` with registry URL
- **Cargo**: `~/.cargo/config.toml` with registry index
- **Python**: `~/.pypirc` with repository URL

Learn more in [credentials_setup.md](guides/credentials_setup.md).

## Next Steps

- **[Full command reference](guides/commands.md)** - All commands, flags, and workflows
- **[Package publishing guide](guides/publishing.md)** - Publishing workflows
- **[Advanced nested repo features](guides/subrepo_management.md)** - Drift detection and sync
