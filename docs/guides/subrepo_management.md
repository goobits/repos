# Nested Repository Management

`repos nested` manages nested Git repositories: directories inside a parent repo
that contain their own `.git` directory.

Nested repositories are not Git submodules. Use `git submodule` for submodules.

## Commands

```bash
repos nested validate
repos nested status
repos nested status --all
repos nested sync <name> --to <commit>
repos nested sync <name> --to <commit> --stash
repos nested update <name>
```

## Drift

Drift happens when the same nested repository appears in multiple parent
repositories at different commits.

`repos sync`, `repos push`, and `repos doctor` report drift when it is detected.

## Validate

```bash
repos nested validate
```

Shows discovered nested repositories and groups shared nested repositories by
remote URL.

## Status

```bash
repos nested status
repos nested status --all
```

Default output is problem-first: drifted nested repositories are shown first.
Use `--all` to include fully synced nested repositories.

## Sync

```bash
repos nested sync shared-lib --to abc1234
repos nested sync shared-lib --to abc1234 --stash
```

Default behavior:

- Syncs clean nested repositories to the requested commit.
- Skips nested repositories with uncommitted changes.
- Does not discard local changes.

Use `--stash` to stash local changes before syncing.

## Update

```bash
repos nested update shared-lib
```

Updates all matching nested repositories to the latest remote commit. Dirty
nested repositories are skipped.

## Recommended Workflow

```bash
repos sync
repos nested status
repos nested sync shared-lib --to abc1234 --stash
repos status
```
