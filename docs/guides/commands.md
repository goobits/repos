# Command Reference

Quick reference for all `repos` commands.

## Table of Contents

- [Command Overview](#command-overview)
- [repos push](#repos-push)
- [repos stage](#repos-stage)
- [repos unstage](#repos-unstage)
- [repos status](#repos-status)
- [repos commit](#repos-commit)
- [repos config](#repos-config)
- [repos publish](#repos-publish)
- [repos audit](#repos-audit)
- [repos subrepo](#repos-subrepo)
  - [repos subrepo validate](#repos-subrepo-validate)
  - [repos subrepo status](#repos-subrepo-status)
  - [repos subrepo sync](#repos-subrepo-sync)
  - [repos subrepo update](#repos-subrepo-update)

## Command Overview

| Command | Purpose |
|---------|---------|
| `push` | Push unpushed commits to remotes |
| `stage` | Stage files matching pattern |
| `unstage` | Unstage files matching pattern |
| `status` | Show staging status across repos |
| `commit` | Commit staged changes |
| `config` | Sync git user.name and email |
| `publish` | Publish packages to registries |
| `audit` | Security scanning and hygiene |
| `subrepo` | Manage nested repositories |

---

## repos push

Push unpushed commits to remotes across all repositories.

`repos push` automatically checks for subrepo drift after pushing, giving you a complete repository health check in one command.

| Flag | Description |
|------|-------------|
| `--force` | Auto-push branches with no upstream |
| `--verbose`, `-v` | Show detailed progress for all repos |
| `--no-drift-check` | Skip subrepo drift check (faster but less complete) |

```bash
repos push                # Push all unpushed commits + check drift
repos push --force        # Auto-create upstream for new branches
repos push --verbose      # Show live progress with tally
repos push --no-drift-check  # Skip drift check for speed
```

**Integrated Health Check**: After pushing, `repos push` automatically checks for subrepo drift and displays a concise summary if any drifted subrepos are found. This gives you a complete picture of your repository health in one command.

---

## repos stage

Stage files matching pattern across all repositories.

```bash
repos stage "*.md"        # Stage all markdown files
repos stage "README.md"   # Stage specific file
repos stage "*"           # Stage all changes
```

---

## repos unstage

Unstage files matching pattern across all repositories.

```bash
repos unstage "*.md"      # Unstage markdown files
repos unstage "*"         # Unstage everything
```

---

## repos status

Show staging status across all repositories.

```bash
repos status              # Show what's staged
```

---

## repos commit

Commit staged changes across all repositories.

| Flag | Description |
|------|-------------|
| `--include-empty` | Include repos with no staged changes (empty commits) |

```bash
repos commit "Fix typos"           # Commit staged changes
repos commit "Bump version" --include-empty
```

---

## repos config

Sync git user.name and email across repositories.

| Flag | Description |
|------|-------------|
| `--name <name>` | Set user name |
| `--email <email>` | Set user email |
| `--from-global` | Use global git config as source |
| `--from-current` | Use current repo's config as source |
| `--force` | Overwrite without prompting |
| `--dry-run` | Preview changes without applying |

**Mutually exclusive:** `--from-global`, `--from-current`, `--name/--email`

```bash
repos config --from-global              # Sync from global config
repos config --from-current             # Sync from current repo
repos config --name "Alice" --email "alice@example.com"
repos config --from-global --dry-run    # Preview
repos config --from-global --force      # No prompts
```

---

## repos publish

Publish packages to npm, Cargo, or PyPI with optional git tagging.

| Flag | Description |
|------|-------------|
| `--dry-run` | Preview without publishing |
| `--tag` | Create and push git tags (e.g., v1.2.3) |
| `--allow-dirty` | Skip clean state check |
| `--all` | Publish all repos (public + private) |
| `--public-only` | Only public repos (default) |
| `--private-only` | Only private repos |

**Mutually exclusive:** `--all`, `--public-only`, `--private-only`

```bash
repos publish                     # Public repos only
repos publish --dry-run           # Preview
repos publish --tag               # Publish + create tags
repos publish my-app my-lib       # Specific repos
repos publish --all --tag         # All repos with tags
```

Learn more in [publishing.md](publishing.md).

---

## repos audit

Security scanning and hygiene checking for repositories.

| Flag | Description |
|------|-------------|
| `--install-tools` | Auto-install TruffleHog without prompting |
| `--verify` | Verify discovered secrets are active |
| `--json` | Output results in JSON format |
| `--interactive` | Choose fixes interactively |
| `--fix-gitignore` | Add .gitignore entries for violations |
| `--fix-large` | Remove large files from history (requires git-filter-repo) |
| `--fix-secrets` | Remove secrets from history |
| `--fix-all` | Apply all available fixes automatically |
| `--dry-run` | Preview fixes without applying |
| `--repos <repo1,repo2>` | Target specific repositories (comma-separated) |

```bash
repos audit                           # Scan all repos
repos audit --install-tools           # Auto-install tools
repos audit --verify                  # Verify secrets are active
repos audit --json                    # JSON output
repos audit --interactive             # Choose fixes
repos audit --fix-gitignore           # Fix gitignore issues
repos audit --fix-all --dry-run       # Preview all fixes
repos audit --repos my-app,my-lib     # Specific repos
```

---

## repos subrepo

Manage nested repository synchronization.

### repos subrepo validate

Discover and validate nested repositories.

```bash
repos subrepo validate    # Show all nested repos
```

### repos subrepo status

Show drift detection for subrepos.

| Flag | Description |
|------|-------------|
| `--all` | Show all subrepos, not just drifted ones |

```bash
repos subrepo status           # Show drifted subrepos
repos subrepo status --all     # Show all subrepos
```

### repos subrepo sync

Sync a subrepo to specific commit across all parents.

| Flag | Description |
|------|-------------|
| `--to <commit>` | Target commit hash (required) |
| `--stash` | Stash uncommitted changes (safe, reversible) |
| `--force` | Force sync, discarding uncommitted changes |

**Note:** If both `--stash` and `--force` are provided, `--stash` takes precedence.

```bash
repos subrepo sync my-lib --to abc1234           # Sync to commit
repos subrepo sync my-lib --to abc1234 --stash   # Safe sync
repos subrepo sync my-lib --to abc1234 --force   # Force sync
```

### repos subrepo update

Update a subrepo to latest origin/main across all parents.

| Flag | Description |
|------|-------------|
| `--force` | Force update with uncommitted changes |

```bash
repos subrepo update my-lib         # Update to origin/main
repos subrepo update my-lib --force # Force update
```

---

**Related Documentation:**
- [Documentation Index](../README.md)
- [Getting Started](../getting_started.md)
- [Publishing Guide](publishing.md)
- [Security Auditing](security_auditing.md)
