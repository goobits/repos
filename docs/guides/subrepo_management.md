# Nested Repository Management

`repos nested` reports and manages Git repositories discovered below another
fleet repository. The inventory includes independent embedded repositories,
registered Git submodules, and linked worktrees, with each checkout type labeled.

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
normalized `origin` identity. Discovery uses the same `.reposignore`-aware fleet
inventory as push/pull/sync, and assigns each checkout to its nearest parent
exactly once.

## Status

```bash
repos nested status
repos nested status --all
```

Default output is problem-first: it details every drifted shared group and
summarizes the rest of the inventory. Use `--all` to enumerate fully synced
shared groups, unique remote-backed repositories, and repositories without an
`origin`.

The summary distinguishes the full fleet repository count, checkout types,
nested copies, and shared/unique groups. Drift comparison applies to every
nested checkout with two or more copies of the same normalized `origin`.

Automatic drift checks after `repos sync`, `repos push`, and `repos pull` always
report shared-group/copy coverage, including a positive zero-drift result. If
inspection fails, the report says the check is incomplete instead of implying
that no drift exists.

## Sync

```bash
repos nested sync shared-lib --to abc1234
repos nested sync shared-lib --to abc1234 --stash
```

Default behavior:

- Verifies the requested commit is available in every eligible copy before changing any checkout.
- Syncs clean nested repositories to the requested commit.
- Skips nested repositories with uncommitted changes.
- Does not discard local changes.

For a submodule, moving the nested checkout also changes the parent repository's
gitlink; commit that parent change before pushing it. Linked worktrees retain
Git's normal worktree checkout constraints.

Use `--stash` to stash local changes before syncing.

## Update

```bash
repos nested update shared-lib
```

Resolves one immutable latest remote commit, then updates every eligible copy to
that same target when the move is a fast-forward. Dirty repositories and
repositories with divergent local commits are skipped for manual review. A
preflight failure aborts ready checkouts before mutation.

## Recommended Workflow

```bash
repos sync
repos nested status
repos nested sync shared-lib --to abc1234 --stash
repos status
```
