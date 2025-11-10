# Troubleshooting Guide

Common issues and solutions for the repos tool.

## Installation Issues

| Error | Cause | Solution |
|-------|-------|----------|
| `repos: command not found` | Binary not in PATH | Add install directory to PATH or move binary to `/usr/local/bin` |
| Build errors during install | Outdated Rust version | Update Rust: `rustup update stable` |
| `Permission denied` | No write access to install location | Use `sudo` or install to user directory (`~/.local/bin`) |

## Publishing Issues

| Error | Cause | Solution |
|-------|-------|----------|
| `not authenticated` or `401 Unauthorized` | Missing registry credentials | Configure [publishing credentials](credentials_setup.md) |
| `uncommitted changes` or `dirty working directory` | Uncommitted files in repo | Commit or stash changes: `git status` to review |
| `tag already exists` | Version tag already published | Delete tag locally and remotely, then retry: `git tag -d v1.0.0 && git push origin :refs/tags/v1.0.0` |
| `not a package` or `no manifest found` | Missing package manifest | Ensure repo has `package.json`, `Cargo.toml`, or `pyproject.toml` |
| `version mismatch` | package.json version differs from git tag | Update package version to match git tag |

## Config Issues

| Error | Cause | Solution |
|-------|-------|----------|
| Config conflicts between repos | Different configs in nested repos | Use `repos config --local` for repo-specific settings |
| Email validation failures | Invalid email format | Use proper format: `user@example.com` |
| `Permission denied` on `.git/config` | Read-only git config | Check file permissions: `chmod 644 .git/config` |
| Config not applied | Wrong config scope | Verify with `git config --list --show-scope` |

## Performance Issues

| Issue | Cause | Solution |
|-------|-------|----------|
| Slow on large monorepos | Expected behavior with many subrepos | Normal operation; tool processes repos concurrently with internal limits |
| Timeout errors after 3 minutes | Repo operation exceeds timeout | Split into smaller operations or check for hanging processes |
| High memory usage | Processing many repos simultaneously | Expected with large repo counts; reduce concurrency if needed |

## Audit Issues

| Error | Cause | Solution |
|-------|-------|----------|
| `TruffleHog not found` | TruffleHog not installed | Run `repos audit --install-tools` to auto-install |
| Too many false positives | TruffleHog sensitivity | Expected; manually review findings and update `.trufflehog-ignore` |
| Need to recover from history rewrite | Used `--rewrite-history` option | Use `git reflog` to find previous commit and reset: `git reset --hard HEAD@{N}` |
| Scan takes too long | Large repository history | Normal for first scan; subsequent scans are faster with verified findings |

## Subrepo Issues

| Error | Cause | Solution |
|-------|-------|----------|
| Subrepos not detected | Missing `.git` directory | Subrepos must have their own `.git` directory (not submodules) |
| Drift false positives | Different remote URLs | Normal if subrepo uses different remote; verify URLs match expectations |
| Sync failures | Uncommitted changes in subrepo | Commit or stash changes in subrepo before syncing |
| `not a valid subrepo` | Directory is git submodule, not subrepo | Subrepos must be independent git repos, not submodules |

## Git Issues

| Error | Cause | Solution |
|-------|-------|----------|
| `push failed: no upstream branch` | Upstream branch not configured | Set upstream: `git push -u origin <branch>` or use `--force` flag |
| `push rejected: non-fast-forward` | Remote has commits not in local | Pull first: `git pull --rebase` or use `repos push --force` (caution) |
| Submodule conflicts | Mixed submodules and subrepos | Use `git submodule` commands for submodules; repos tool handles subrepos |
| Worktree detection issues | Git worktree not recognized | Ensure worktree is properly configured: `git worktree list` |
| `not a git repository` | Command run outside git repo | Navigate to git repository root directory |

## General Debugging

### Enable Verbose Output
```bash
repos <command> --verbose  # Show detailed operation logs
repos status -v            # Short form for verbose
```

### Check Git State Manually
```bash
git status                 # Check working directory status
git log --oneline -5       # Review recent commits
git remote -v              # Verify remote URLs
git tag                    # List all tags
```

### Common Diagnostic Commands
```bash
repos status               # Check status of all repos
git config --list          # View git configuration
which repos                # Verify binary location
repos --version            # Check installed version
```

### Still Having Issues?

1. Check that git works normally: `git --version` and `git status`
2. Verify you're in a git repository root directory
3. Ensure the operation is valid for your repository type
4. Review command syntax in the main documentation
5. Check file permissions on `.git` directory and config files

### Getting Help

For additional support, consult the main documentation:
- [README.md](../README.md) - Main documentation
- [getting_started.md](../getting_started.md) - Quick start guide
- [credentials_setup.md](credentials_setup.md) - Authentication setup

---

**Related Documentation:**
- [Documentation Index](../README.md)
- [Getting Started](../getting_started.md)
- [Commands Reference](commands.md)
- [Security Auditing](security_auditing.md)
