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

### How do I target specific repositories instead of all of them?

Most commands accept repository names as arguments:
```bash
repos publish my-app my-lib        # Only these repos
repos audit --repos my-app,my-lib  # Comma-separated for audit
```

**More questions?** See [Publishing Guide](guides/publishing.md), [Security Auditing](guides/security_auditing.md), [Subrepo Management](guides/subrepo_management.md), or [Troubleshooting](guides/troubleshooting.md).

## Migrating from Shell Scripts

Common shell script patterns and their `repos` equivalents:

| Task | Shell Script | repos | Advantage |
|------|--------------|-------|-----------|
| Push all repos | `for d in */; do (cd "$d" && git push); done` | `repos push` | Concurrent + drift check |
| Stage files | `for d in */; do (cd "$d" && git add "*.md"); done` | `repos stage "*.md"` | Pattern matching, atomic |
| Commit all | `find . -name .git -execdir git commit -m "msg" \;` | `repos commit "msg"` | Staged-only, summary |
| Git config | `for d in */; do (cd "$d" && git config user.name "Alice"); done` | `repos config --name "Alice"` | Sync from global |

### Quick Migration Steps

1. **Identify your scripts** - Find all git automation in your workflow
2. **Map to repos commands** - Use table above for common patterns
3. **Test with dry-run** - Use `repos publish --dry-run` to preview
4. **Replace gradually** - One workflow at a time, keep scripts as backup

### Migrating from Other Tools

**From git-multi:**
- repos adds: concurrent operations, drift detection, publishing support, security scanning
- Automatic repository discovery (no manual config)

**From Google's repo tool:**
- repos uses auto-discovery instead of manifests
- No XML configuration needed
- Cross-language package publishing built-in

## Next Steps

- **[Full command reference](guides/commands.md)** - All commands, flags, and workflows
- **[Package publishing guide](guides/publishing.md)** - Publishing workflows
- **[Advanced nested repo features](guides/subrepo_management.md)** - Drift detection and sync
