# Glossary

Quick reference for all `repos` terminology, flags, and concepts. Use this page to look up unfamiliar terms while reading other documentation, or browse by category to learn what `repos` can do.

Entries link to detailed guides for deeper learning.

---

## Commands

### Core Commands

**`repos push`** - Push unpushed commits to remotes across all repositories
→ [Commands Reference](guides/commands.md#repos-push)

**`repos stage`** - Stage files matching pattern across all repositories
→ [Commands Reference](guides/commands.md#repos-stage)

**`repos commit`** - Commit staged changes across all repositories
→ [Commands Reference](guides/commands.md#repos-commit)

**`repos config`** - Synchronize git user.name and email across repositories
→ [Commands Reference](guides/commands.md#repos-config)

**`repos publish`** - Publish packages to registries (npm, Cargo, PyPI)
→ [Publishing Guide](guides/publishing.md)

**`repos audit`** - Security scanning and repository hygiene checking
→ [Security Auditing](guides/security_auditing.md)

**`repos subrepo`** - Manage nested repository synchronization
→ [Subrepo Management](guides/subrepo_management.md)

## Common Flags

### Universal Flags

**`--dry-run`** - Preview changes without applying them
Used by: `config`, `publish`, `audit`

**`--force`** - Force operation, bypassing safety checks
Used by: `push`, `config`, `subrepo sync`, `subrepo update`

**`--no-drift-check`** - Skip subrepo drift check in push (faster but less complete)
Used by: `push`

**`--verbose` / `-v`** - Show detailed operation logs
Used by: Most commands

**`--repos <repo1,repo2>`** - Target specific repositories (comma-separated)
Used by: `audit`

### Publishing Flags

**`--tag`** - Create and push git tags after successful publish
→ [Publishing Guide](guides/publishing.md#flags)

**`--allow-dirty`** - Skip clean working directory check
→ [Publishing Guide](guides/publishing.md#flags)

**`--all`** - Publish all repos (public + private)
→ [Publishing Guide](guides/publishing.md#flags)

**`--public-only`** - Only publish public repos (default behavior)
→ [Publishing Guide](guides/publishing.md#flags)

**`--private-only`** - Only publish private repos
→ [Publishing Guide](guides/publishing.md#flags)

### Config Flags

**`--from-global`** - Use global git config as source
→ [Commands Reference](guides/commands.md#repos-config)

**`--from-current`** - Use current repo's config as source
→ [Commands Reference](guides/commands.md#repos-config)

**`--name <name>`** - Set user name
→ [Commands Reference](guides/commands.md#repos-config)

**`--email <email>`** - Set user email
→ [Commands Reference](guides/commands.md#repos-config)

### Audit Flags

**`--install-tools`** - Auto-install TruffleHog without prompting
→ [Security Auditing](guides/security_auditing.md#trufflehog-secret-scanning)

**`--verify`** - Verify discovered secrets are active (exits 1 if found)
→ [Security Auditing](guides/security_auditing.md#secret-detection)

**`--fix-gitignore`** - Add .gitignore entries for violations (safe)
→ [Security Auditing](guides/security_auditing.md#automated-fixes)

**`--fix-large`** - Remove large files from history (destructive)
→ [Security Auditing](guides/security_auditing.md#automated-fixes)

**`--fix-secrets`** - Remove secrets from history (destructive)
→ [Security Auditing](guides/security_auditing.md#automated-fixes)

**`--fix-all`** - Apply all available fixes automatically
→ [Security Auditing](guides/security_auditing.md#automated-fixes)

**`--interactive`** - Choose fixes interactively
→ [Security Auditing](guides/security_auditing.md#interactive-mode)

**`--json`** - Output results in JSON format
→ [Security Auditing](guides/security_auditing.md#json-output)

### Subrepo Flags

**`--to <commit>`** - Target commit hash for sync (required for `sync`)
→ [Subrepo Management](guides/subrepo_management.md#repos-subrepo-sync)

**`--stash`** - Stash uncommitted changes (safe, reversible)
→ [Subrepo Management](guides/subrepo_management.md#repos-subrepo-sync)

**`--all`** - Show all subrepos, not just drifted ones
→ [Subrepo Management](guides/subrepo_management.md#repos-subrepo-status)

## Concepts

### Subrepo Terms

**Subrepo** - Nested Git repository within a parent repo (has its own `.git` directory)
→ [Subrepo Management](guides/subrepo_management.md#what-are-subrepos)

**Drift** - When the same subrepo is at different commits across parent repositories
→ [Subrepo Management](guides/subrepo_management.md#drift-detection)

**Sync Score** - Percentage (0-100%) showing how well synchronized subrepo instances are
→ [Subrepo Management](guides/subrepo_management.md#sync-score)

**Sync Target** - Latest clean commit recommended for synchronization (indicated by → arrow)
→ [Subrepo Management](guides/subrepo_management.md#visual-indicators)

### Security Terms

**Secret Scanning** - Detecting exposed credentials and API keys in git history
→ [Security Auditing](guides/security_auditing.md#trufflehog-secret-scanning)

**Hygiene Checking** - Detecting improperly committed files (gitignore violations, large files)
→ [Security Auditing](guides/security_auditing.md#hygiene-checking)

**Gitignore Violation** - Files tracked by git that match `.gitignore` patterns
→ [Security Auditing](guides/security_auditing.md#1-gitignore-violations)

**Universal Bad Patterns** - Commonly ignored files that should never be committed
→ [Security Auditing](guides/security_auditing.md#2-universal-bad-patterns)

**Large Files** - Files exceeding 1MB threshold in git history
→ [Security Auditing](guides/security_auditing.md#3-large-files)

**History Rewriting** - Permanently modifying git history to remove files/secrets
→ [Security Auditing](guides/security_auditing.md#destructive-history-rewriting)

**Verification Mode** - Testing if secrets are currently active (slower, exits 1 if found)
→ [Security Auditing](guides/security_auditing.md#secret-detection)

### Publishing Terms

**Visibility Filtering** - Only publishing public repos by default (configurable with flags)
→ [Publishing Guide](guides/publishing.md#flags)

**Package Manager** - npm (JavaScript), Cargo (Rust), or PyPI (Python)
→ [Publishing Guide](guides/publishing.md#how-it-works)

**Registry** - Package hosting service (npmjs.org, crates.io, pypi.org, or private)
→ [Credentials Setup](guides/credentials_setup.md#private-registries)

**Token Authentication** - Using API tokens instead of passwords for publishing
→ [Credentials Setup](guides/credentials_setup.md)

### Git Terms

**Working Directory** - Current state of files in repository
→ [Troubleshooting](guides/troubleshooting.md#publishing-issues)

**Upstream Branch** - Remote branch that local branch tracks
→ [Troubleshooting](guides/troubleshooting.md#git-issues)

**Force Push** - Overwriting remote history (use `--force-with-lease` for safety)
→ [Security Auditing](guides/security_auditing.md#destructive-history-rewriting)

**Staged Changes** - Files marked for inclusion in next commit
→ [Commands Reference](guides/commands.md#repos-stage)

## Output Indicators

### Status Indicators

**🟢** - Success / Clean state / Published
**🟡** - Warning / Uncommitted changes / Skipped
**🟠** - Already published / Up-to-date
**🔴** - Error / Failed / Verified secrets found

### Subrepo Indicators

**→** - Arrow pointing to recommended sync target (latest clean commit)
**✅ clean** - No uncommitted changes
**⚠️ uncommitted** - Has uncommitted changes
**⬆️ LATEST** - Absolute newest commit
**(outdated)** - Commit is older than the latest

---

**Related Documentation:**
- [Documentation Index](README.md)
- [Getting Started](getting_started.md)
- [Commands Reference](guides/commands.md)
