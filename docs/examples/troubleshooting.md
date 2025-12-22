# Troubleshooting Common Issues

This guide covers common issues you might encounter when using `repos` and how to resolve them.

## General Issues

### "Too many open files"

**Symptoms:** Commands fail with IO errors when processing many repositories.

**Cause:** The operating system limit for open file descriptors has been reached. `repos` processes repositories in parallel, which can consume many file descriptors.

**Solution:**
Increase your user limit:
```bash
ulimit -n 4096
```
Or limit concurrency in `repos`:
```bash
repos push --jobs 4
```

### "Rate limit exceeded" (GitHub/GitLab)

**Symptoms:** `403 Forbidden` or "Secondary Rate Limit" errors during push/pull/audit.

**Cause:** Sending too many requests to the git host in a short period.

**Solution:**
1. Reduce concurrency: `repos pull --jobs 2`
2. Ensure you are authenticated (anonymous requests have lower limits).
3. Wait a few minutes before retrying.

## Git Operations

### "Detached HEAD" warnings

**Symptoms:** `repos pull` skips repositories with "detached HEAD" warning.

**Cause:** The repository is checked out to a specific commit, not a branch. `repos` only pulls for branches to avoid unexpected changes.

**Solution:**
Checkout a branch in that repository:
```bash
git checkout main
```

### "Merge conflict" or "Diverged"

**Symptoms:** `repos pull` fails with "diverged: X ahead, Y behind".

**Cause:** You have local commits that conflict with remote commits.

**Solution:**
You must resolve this manually. Go to the repository and merge or rebase:
```bash
cd path/to/repo
git pull --rebase
# Resolve conflicts...
git rebase --continue
```

## Subrepos

### "Uncommitted changes" during sync

**Symptoms:** `repos subrepo sync` skips a repo.

**Cause:** You have modified files in the subrepo. Syncing would overwrite them.

**Solution:**
- **Safe:** Use `--stash` to auto-stash changes: `repos subrepo sync name --to commit --stash`
- **Destructive:** Use `--force` to discard changes: `repos subrepo sync name --to commit --force`

## Security Audit

### "git-filter-repo not found"

**Symptoms:** `repos audit hygiene --fix-large` fails.

**Cause:** The `git-filter-repo` tool is not installed. It is required for history rewriting.

**Solution:**
Install it using Python pip (recommended):
```bash
pip install git-filter-repo
```
Or your system package manager.

### "History rewrite requires force push"

**Symptoms:** After running fixes, `repos push` is rejected.

**Cause:** History rewriting changes commit hashes. You must force-push to update the remote.

**Solution:**
```bash
repos push --force
```
**Warning:** This will overwrite history on the remote. Coordinate with your team.
