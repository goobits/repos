# Command Reference

`repos` manages a fleet of Git repositories from one directory tree.

The CLI is intent-first for daily work and keeps granular Git controls available
when you need them.

## Overview

```text
repos - Fleet-scale Git orchestration for humans

USAGE:
  repos <command> [options]

EVERYDAY:
  status      Understand repository state
  save        Stage tracked changes, commit, and push
  sync        Fetch, rebase safe repositories, and report nested drift

CONTROL:
  stage       Stage matching files
  unstage     Unstage matching files
  commit      Commit currently staged changes
  push        Push unpushed commits
  pull        Pull remote changes

MAINTENANCE:
  audit       Scan for secrets and hygiene issues
  publish     Publish detected packages
  doctor      Diagnose remotes, upstreams, worktrees, and nested drift

ADVANCED:
  nested      Manage nested repository drift
  config      Sync Git identity/config
```

## Everyday

### `repos status`

Show repository state across the fleet.

```bash
repos status
```

Reports staged, unstaged, and untracked changes per repository.

### `repos save`

Stage tracked changes, commit, and push in one command.

```bash
repos save "Update docs"
```

Safe default:

- Stages tracked modifications and deletions only.
- Does not stage untracked files by default.
- Commits repositories with staged changes.
- Pushes successful commits.
- Skips branches without upstream unless `--auto-upstream` is passed.

Options:

| Option | Description |
|---|---|
| `-u`, `--include-untracked` | Include untracked files |
| `-a`, `--all` | Stage all non-ignored changes |
| `--auto-upstream` | Set upstream for branches without tracking |
| `--dry-run` | Print planned save actions without mutating repositories |

Examples:

```bash
repos save "Update docs"
repos save "Add assets" --include-untracked
repos save "Initial project state" --all
repos save "Publish branch" --auto-upstream
repos save "Preview save" --dry-run
```

### `repos sync`

Fetch and pull safe repositories using rebase.

```bash
repos sync
```

Default behavior:

- Fetches remotes.
- Pulls with rebase.
- Skips dirty repositories instead of stashing implicitly.
- Reports nested repository drift.
- Leaves granular pull behavior available through `repos pull`.

Options:

| Option | Description |
|---|---|
| `-v`, `--verbose` | Show detailed progress |
| `-c`, `--show-changes` | Show file changes in dirty repositories |
| `--no-drift-check` | Skip nested drift check |

Advanced options are hidden from main help but still available:

| Option | Description |
|---|---|
| `-j`, `--jobs <N>` | Limit concurrency |
| `--sequential` | Run one repository at a time |

## Control

### `repos stage`

Stage files matching a pattern across repositories.

```bash
repos stage "*.md"
repos stage "README.md"
repos stage "*"
```

### `repos unstage`

Unstage files matching a pattern across repositories.

```bash
repos unstage "*.md"
repos unstage "*"
```

### `repos commit`

Commit currently staged changes.

```bash
repos commit "Fix typos"
repos commit "Bump version" --include-empty
```

Options:

| Option | Description |
|---|---|
| `--include-empty` | Create empty commits in repositories without staged changes |

### `repos push`

Push unpushed commits.

```bash
repos push
repos push --auto-upstream
```

Options:

| Option | Description |
|---|---|
| `--auto-upstream` | Set upstream for branches without tracking |
| `-v`, `--verbose` | Show detailed progress |
| `-c`, `--show-changes` | Show file changes in dirty repositories |
| `--no-drift-check` | Skip nested drift check |

Advanced options are hidden from main help but still available:

| Option | Description |
|---|---|
| `-j`, `--jobs <N>` | Limit concurrency |
| `--sequential` | Run one repository at a time |

### `repos pull`

Granular pull command.

```bash
repos pull
repos pull --rebase
```

Options:

| Option | Description |
|---|---|
| `--rebase` | Use `git pull --rebase` |
| `-v`, `--verbose` | Show detailed progress |
| `-c`, `--show-changes` | Show file changes in dirty repositories |
| `--no-drift-check` | Skip nested drift check |

Advanced options are hidden from main help but still available:

| Option | Description |
|---|---|
| `-j`, `--jobs <N>` | Limit concurrency |
| `--sequential` | Run one repository at a time |

## Maintenance

### `repos audit`

Scan for secrets and repository hygiene issues.

```bash
repos audit
repos audit --verify
repos audit --json
repos audit --fix-gitignore
repos audit --fix-all --dry-run
```

Options:

| Option | Description |
|---|---|
| `--install-tools` | Install required tools without prompting |
| `--verify` | Verify discovered secrets are active |
| `--json` | Output JSON |
| `--interactive` | Choose fixes interactively |
| `--fix-gitignore` | Add missing `.gitignore` entries |
| `--fix-large` | Remove large files from history |
| `--fix-secrets` | Remove secrets from history |
| `--fix-all` | Apply all available fixes |
| `--dry-run` | Preview fixes |
| `--repos <repo1,repo2>` | Target specific repositories |

### `repos publish`

Publish detected packages to registries.

```bash
repos publish
repos publish --dry-run
repos publish --tag
repos publish my-app my-lib
```

Options:

| Option | Description |
|---|---|
| `--dry-run` | Preview without publishing |
| `--tag` | Create and push Git tags after publish |
| `--allow-dirty` | Allow publishing dirty repositories |
| `--all` | Publish public and private repositories |
| `--public-only` | Publish public repositories only |
| `--private-only` | Publish private repositories only |

### `repos doctor`

Diagnose common fleet blockers without mutating anything.

```bash
repos doctor
```

Checks:

- Detached HEADs.
- Missing remotes.
- Missing upstream tracking.
- Dirty worktrees.
- Conflicts.
- Nested repository drift.

## Advanced

### `repos nested`

Manage nested repository drift.

```bash
repos nested validate
repos nested status
repos nested status --all
repos nested sync my-lib --to abc1234
repos nested sync my-lib --to abc1234 --stash
repos nested update my-lib
```

Subcommands:

| Subcommand | Description |
|---|---|
| `validate` | Validate nested repository setup |
| `status` | Show nested drift |
| `sync` | Sync a nested repository to a commit |
| `update` | Update a nested repository to latest remote commit |

### `repos config`

Sync Git identity across repositories.

```bash
repos config --from-global
repos config --from-current
repos config --name "Alice" --email "alice@example.com"
repos config --from-global --dry-run
repos config --from-global --yes
```

Options:

| Option | Description |
|---|---|
| `--name <name>` | Set Git user name |
| `--email <email>` | Set Git user email |
| `--from-global` | Use global Git config as source |
| `--from-current` | Use current repository config as source |
| `--yes` | Apply without prompting |
| `--dry-run` | Preview changes |
