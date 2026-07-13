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
| Config conflicts between repos | Different configs in nested repos | Use `git config --local` directly in specific repos to set repo-specific values |
| Email validation failures | Invalid email format | Use proper format: `user@example.com` |
| `Permission denied` on `.git/config` | Read-only git config | Check file permissions: `chmod 644 .git/config` |
| Config not applied | Wrong config scope | Verify with `git config --list --show-scope` |

## Performance Issues

| Issue | Cause | Solution |
|-------|-------|----------|
| Slow on large monorepos | Expected behavior with many nested repos | Normal operation; tool processes repos concurrently with internal limits |
| Timeout errors after 3 minutes | Repo operation exceeds timeout | Split into smaller operations or check for hanging processes |
| High memory usage | Processing many repos simultaneously | Expected with large repo counts; reduce concurrency if needed |

## Audit Issues

| Error | Cause | Solution |
|-------|-------|----------|
| `TruffleHog not found` | TruffleHog not installed | Run `repos audit --install-tools` to auto-install |
| Suspected false positives | TruffleHog classified test/revoked data as a secret | Review the finding manually; `repos` does not maintain an ignore file |
| Need to recover from history rewrite | Used history rewriting options (`--fix-large`, `--fix-secrets`, or `--fix-all`) | Use `git reflog` to find previous commit and reset: `git reset --hard HEAD@{N}` |
| Scan takes too long | Large repository history | Narrow the run with `--repos` or inspect the repository history separately |
| `audit incomplete` | A secret or hygiene scanner could not inspect one or more repositories | Fix the reported tool/repository error and rerun; an incomplete scan is never reported as clean |
| `git fetch failed` before a history fix | Configured upstream is inaccessible | Restore access or correct the remote before rewriting history |

## Nested Repository Issues

| Error | Cause | Solution |
|-------|-------|----------|
| Nested repos not detected | Missing `.git` directory | Nested repos must have their own `.git` directory |
| Drift false positives | Different remote URLs | Normal if a nested repo uses a different remote; verify URLs match expectations |
| Nested repository name is ambiguous | The same directory name points at different remotes | Rename one nested checkout or operate after making the target name unique |
| Sync failures | Uncommitted changes in nested repo | Commit or stash changes before syncing |
| `not a valid nested repo` | Directory is Git submodule, not nested repo | Use `git submodule` commands for submodules |

## Git Issues

| Error | Cause | Solution |
|-------|-------|----------|
| `push failed: no upstream branch` | Upstream branch not configured | Use `repos push --auto-upstream` |
| `push rejected: non-fast-forward` | Remote has commits not in local | Use `repos sync` or resolve the branch manually |
| `Could not read from remote repository` | The configured SSH key is missing or unauthorized | Test the configured URL with `git ls-remote <url>`, then install/authorize the correct key |
| HTTPS authentication failure | The configured HTTPS credential is missing or expired | Refresh the credential-manager or host CLI login; `repos` does not rewrite HTTPS remotes to SSH |
| Submodule conflicts | Mixed submodules and nested repos | Use `git submodule` commands for submodules; repos handles nested repos |
| Worktree detection issues | Git worktree not recognized | Ensure worktree is properly configured: `git worktree list` |
| `not a git repository` | A discovered repository is damaged or a direct Git command used the wrong directory | Run `repos status --failed` to identify it and repair its `.git` metadata |

Git credential prompts are disabled during fleet operations so one inaccessible
repository cannot hang the whole run. The command records the repository as
failed and exits nonzero. The remote URL and transport in `.git/config` are
used as configured; custom `GIT_SSH_COMMAND` values are preserved. `repos
doctor` warns when a remote uses HTTP(S), without failing solely because of that
transport.

For a GitHub repository in an SSH-only setup:

```bash
git remote set-url origin git@github.com:OWNER/REPOSITORY.git
git ls-remote origin
```

## General Debugging

### Enable Verbose Output
```bash
repos push --verbose       # Show detailed operation logs
repos push -v              # Short form for verbose
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
repos status --failed      # Show repositories whose status inspection failed
repos doctor               # Probe remotes, upstreams, worktrees, and nested drift
git config --list          # View git configuration
git remote -v              # Confirm each configured transport and URL
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
