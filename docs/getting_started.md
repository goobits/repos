# Getting Started

Fleet-scale Git orchestration across multiple repositories.

Instead of acting as a human `for` loop, use intent-driven commands:

```bash
repos status
repos save "Update docs"
repos sync
```

## Installation

```bash
./install.sh  # See installation.md for alternatives
```

## Quick Tour

Run `repos` in a directory tree that contains Git repositories.

### Understand State

```bash
repos status
```

Use this first when you want to know which repositories are clean, dirty, staged,
untracked, ahead, or behind.

### Save Work

```bash
repos save "Update docs"
```

This stages tracked modifications and deletions, commits them, and pushes the
result. It does not stage untracked files unless you opt in, which prevents
accidental commits of scratch files, generated output, secrets, or local config:

```bash
repos save "Add new docs" --include-untracked
```

### Sync Repositories

```bash
repos sync
```

This fetches remotes, pulls with rebase where safe, and reports nested repository
drift. Dirty repositories are skipped instead of being stashed implicitly.

Example drift summary:

```text
🔴 NESTED DRIFT (1)
auth: 2 instances at different commits
  → 105ce4e  app       ✅ clean  ⬆️ LATEST
    2f13c23  website   ✅ clean  (outdated)
    Sync: repos nested sync auth --to 105ce4e
```

### Granular Git Control

```bash
repos stage "*.md"
repos commit "Update docs"
repos push
repos pull --rebase
```

Use these when you need explicit Git-shaped operations instead of the daily
`save` and `sync` workflows.

### Sync Git Config

```bash
repos config --from-global
repos config --name "Alice" --email "alice@example.com"
```

## Common Workflows

### Daily Save

```bash
repos status
repos save "Update docs"
```

Preview first when you are unsure:

```bash
repos save "Update docs" --dry-run
```

### Include New Files

```bash
repos save "Add examples" --include-untracked
```

### Publish Packages

```bash
repos publish --dry-run
repos publish --tag
```

Publishing requires authentication. See [credentials setup](guides/credentials_setup.md)
to configure npm, Cargo, or PyPI credentials.

### Diagnose Blockers

```bash
repos doctor
```

`doctor` checks detached HEADs, missing remotes, missing upstreams, dirty
worktrees, conflicts, and nested drift.

## FAQ

### When should I use `repos` instead of manual Git commands?

Use `repos` when you need the same operation across multiple repositories.
Instead of running `cd repo1 && git push && cd ../repo2 && git push`, run one
fleet command.

### Should I use `sync` or `pull`?

Use `repos sync` for daily work. It fetches, rebases safe repositories, and
reports nested drift. Use `repos pull` when you specifically want the granular
Git-shaped command.

### Does `repos` work with Git submodules?

`repos` treats submodules as separate repositories. The `nested` commands are
for nested repos with independent `.git` directories, not Git submodules. Use
`git submodule` commands for submodule management.

### Can I use `repos` in a monorepo?

Yes. `repos` works with any directory structure containing multiple Git
repositories. It discovers all repos recursively and operates on them
concurrently.

### How do I target specific repositories instead of all of them?

Some commands support explicit repository arguments or filters:

```bash
repos publish my-app my-lib
repos audit --repos my-app,my-lib
```

## Migrating from Shell Scripts

| Task | Shell Script | repos |
|---|---|---|
| Save all tracked work | `for d in */; do (cd "$d" && git add -u && git commit -m "msg" && git push); done` | `repos save "msg"` |
| Push all repos | `for d in */; do (cd "$d" && git push); done` | `repos push` |
| Stage files | `for d in */; do (cd "$d" && git add "*.md"); done` | `repos stage "*.md"` |
| Commit all staged work | `find . -name .git -execdir git commit -m "msg" \;` | `repos commit "msg"` |
| Git config | `for d in */; do (cd "$d" && git config user.name "Alice"); done` | `repos config --name "Alice"` |

## Next Steps

- [Full command reference](guides/commands.md)
- [Package publishing guide](guides/publishing.md)
- [Nested repository management](guides/subrepo_management.md)
