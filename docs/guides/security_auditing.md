# Security Auditing and Hygiene

Comprehensive security scanning and repository hygiene checking with automated fixes.

History objects and secret-scanner JSON are processed as bounded streams. The
large-file check examines every reachable object and retains the complete
finding set; streaming changes memory use, not audit coverage.

## Table of Contents

- [Overview](#overview)
- [TruffleHog Secret Scanning](#trufflehog-secret-scanning)
  - [Installation](#installation)
  - [Secret Detection](#secret-detection)
  - [Output](#output)
  - [Reviewing Findings](#reviewing-findings)
- [Hygiene Checking](#hygiene-checking)
  - [Gitignore Violations](#1-gitignore-violations)
  - [Universal Bad Patterns](#2-universal-bad-patterns)
  - [Large Files](#3-large-files)
- [Automated Fixes](#automated-fixes)
  - [Fix Flags](#fix-flags)
  - [Fix Workflows](#fix-workflows)
  - [Interactive Mode](#interactive-mode)
- [Additional Flags](#additional-flags)
- [Security Best Practices](#security-best-practices)
- [Examples](#examples)

## Overview

The `repos audit` command combines TruffleHog secret scanning with repository hygiene checking to identify security issues and improperly committed files across all repositories.

Audits fail closed: if TruffleHog, Git history inspection, or hygiene scanning
cannot inspect a repository, the command exits nonzero instead of treating that
repository as clean.

**Concurrency:**
- TruffleHog scanning: 1 concurrent (CPU-intensive)
- Hygiene checking: 3 concurrent

```bash
repos audit                    # Scan all repos
repos audit --install-tools    # Auto-install TruffleHog
repos audit --verify           # Verify findings; fail closed on active/unknown
repos audit --fix-all          # Apply all fixes
```

---

## TruffleHog Secret Scanning

### Installation

TruffleHog must be installed before running audits. Install manually or use `--install-tools`:

```bash
# Auto-install
repos audit --install-tools

# Manual installation
brew install trufflesecurity/trufflehog/trufflehog  # macOS
```

On Linux, install a trusted TruffleHog release for your platform. Automatic
installation downloads the upstream installer to a private temporary file and
executes it only when its SHA-256 checksum matches the version trusted by this
release of `repos`.

### Secret Detection

Scans git history for exposed credentials and API keys:

```bash
repos audit                    # Offline detection; no verifier/API calls
repos audit --verify           # Classify verified, unverified, and unknown
```

The default scan passes `--no-verification` to TruffleHog and reports its
unverified detections. Verification mode can contact external services and is
slower. It retains every result class and exits nonzero for either a verified
secret or an inconclusive/unknown verification result. Any scanner failure also
makes the audit incomplete and nonzero.

### Output

```
🟢 my-app      no secrets
🟡 api-server  3 secrets (unverified)
🔴 web-app     2 secrets (1 verified)
🟡 worker      1 secret (1 verification unknown)

═══════════════════════════════════════════════════════════════════
🔍 SECRET SCANNING RESULTS
═══════════════════════════════════════════════════════════════════
🔴 VERIFIED SECRETS FOUND (1)
   These secrets are confirmed to be active and should be rotated immediately!

🟠 UNKNOWN SECRET VERIFICATION (1)
   Verification failed; treat these findings as unsafe until reviewed.

📊 SECRETS BY TYPE
   2 × GitHub
   1 × AWS
   1 × Slack

═══════════════════════════════════════════════════════════════════
```

### Reviewing Findings

Review every finding before changing history. Rotate active credentials first,
then decide whether the affected content should be removed. `repos` does not
maintain a project-specific ignore file or silently suppress scanner output.

---

## Hygiene Checking

Detects three types of violations:

### 1. Gitignore Violations

Files tracked by git that match `.gitignore` patterns:

```bash
git ls-files -i -c --exclude-standard
```

### 2. Universal Bad Patterns

Commonly ignored files that should never be committed:

| Pattern | Description |
|---------|-------------|
| `node_modules/` | Node.js dependencies |
| `vendor/` | Vendored dependencies |
| `dist/`, `build/` | Build artifacts |
| `target/debug/`, `target/release/` | Rust build outputs |
| `__pycache__/`, `.venv/` | Python artifacts |
| `.env` | Environment variables |
| `*.log`, `*.tmp`, `*.cache` | Temporary files |
| `.DS_Store`, `Thumbs.db` | OS metadata |
| `*.key`, `*.pem`, `*.p12`, `*.jks` | Private keys/certificates |
| `.idea/`, `.vscode/settings.json` | IDE configs |

### 3. Large Files

Files exceeding the **1MB threshold** anywhere in reachable Git history:

```bash
git rev-list --objects --all | git cat-file --batch-check
```

The audit retains every matching historical path; any display formatting limit
is never reused as a fix limit.

### Hygiene Output

```
🟡 HYGIENE VIOLATIONS (2)
   ├─ my-app               ~/repos/my-app          # 5 violations (2 gitignore, 1 patterns, 2 large)
   └─ web-app              ~/repos/web-app         # 3 violations (3 gitignore, 0 patterns, 0 large)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

---

## Automated Fixes

### Fix Flags

| Flag | Action | Risk Level |
|------|--------|------------|
| `--fix-gitignore` | Add patterns to `.gitignore` | Safe |
| `--fix-large` | Remove large files from history | Destructive |
| `--fix-secrets` | Remove/redact secrets from history | Destructive |
| `--fix-all` | Apply all fixes | Destructive |
| `--dry-run` | Preview without applying | None |
| `--interactive` | Choose fixes interactively | Varies |

### Fix Workflows

#### Safe: Update .gitignore Only

```bash
repos audit --fix-gitignore
```

Adds missing patterns to `.gitignore`:
- Groups patterns intelligently (`*.log` instead of individual files)
- Preserves existing `.gitignore` content
- Creates commit: `chore: Update .gitignore`
- **Does not untrack files** (they remain in git)

#### Destructive: Apply Every Fix

```bash
repos audit --fix-all
```

This combines `.gitignore` cleanup with every selected history rewrite:

- Updates `.gitignore`
- Untracks affected files with literal pathspecs while keeping them locally
- Removes large paths and secrets from history
- Files untracked by the `.gitignore` step remain in the working directory

Use `--interactive` when you want untracking without automatically selecting
the history-rewrite fixes.

#### Destructive: History Rewriting

```bash
# Preview first
repos audit --fix-large --fix-secrets --dry-run

# Apply with confirmation
repos audit --fix-large --fix-secrets
```

**Requirements:**
- Git 2.36 or newer and `git-filter-repo` must be installed. On macOS, use
  `brew install git-filter-repo`; otherwise use a trusted package source.
- Repository must be clean (no uncommitted changes)
- A repository with remotes must be on an attached branch with a configured,
  reachable upstream
- Local `HEAD` must not be behind that upstream
- The upstream fetch URL must comply with `repos.transportPolicy`
- Record the reviewed remote URL before starting because `git-filter-repo`
  normally removes `origin`
- All collaborators must re-clone after rewritten refs are published

**What happens:**
1. Inspects the exact upstream fetch transport, refreshes its branches and
   tags, and refuses a checkout that is behind.
2. Rebuilds one validated plan from the refreshed reachable history.
3. Creates and verifies a full-ref backup at
   `.git/repos-backups/.../before.bundle` (mode `0600` on Unix).
4. Runs one `git filter-repo` rewrite. Complete secret values are redacted only
   in historical blobs belonging to the reported path; findings without a safe
   complete value remove that affected path instead.
5. Re-runs the selected secret and large-object scans. A remaining finding is a
   failed fix and the error repeats the recovery-bundle path.
6. Leaves all remote publication to the operator. The command never force-pushes
   or restores a remote removed by `git-filter-repo`.

**Recovery:** clone the exact bundle path printed by the command into a clean
directory. This recovers the pre-rewrite `HEAD`, branches, and tags without
overwriting the rewritten checkout.

```bash
git clone /path/from/output/before.bundle recovered-repository
```

The bundle also contains the removed sensitive history. Keep its `0600`
permissions, store it as sensitive material, and delete it only after the agreed
recovery window.

**Publication:** rotate exposed credentials first. Then inspect every local and
remote ref from the pre-rewrite inventory, coordinate a maintenance window, and
restore only a reviewed remote URL if `origin` was removed. Publish every
rewritten branch and tag using your hosting provider's guarded force-update
procedure. An ordinary `git push` or a force-push of only the current branch is
incomplete; remote-only branches, tags, pull-request refs, and cached artifacts
may need separate host-specific cleanup. Require collaborators to re-clone after
publication.

### Interactive Mode

```bash
repos audit --interactive
```

Prompts for each fix type:
```
Add missing .gitignore patterns? [y/N]: y
Also untrack the affected files while keeping them locally? [y/N]: n
Remove large files from Git history? [y/N]: y
Remove secrets from Git history? [y/N]: n
```

Only finding types present in the scan are offered. An EOF or any answer other
than `y`/`yes` declines that choice. If a destructive choice is selected, the
normal history-rewrite summary and final `yes` confirmation follow.

---

## Additional Flags

| Flag | Description |
|------|-------------|
| `--json` | Output results in JSON format |
| `--repos <repo1,repo2>` | Target specific repositories (comma-separated) |

`--interactive` cannot be combined with JSON or explicit fix flags; it owns the
selection prompts for that run.

### JSON Output

```bash
repos audit --json
```

Standard output contains exactly one JSON document. Progress and fix diagnostics
use standard error, so redirecting stdout produces a parseable report.

```json
{
  "truffle": {
    "summary": {
      "total_repos_scanned": 5,
      "repos_with_secrets": 2,
      "total_secrets": 3,
      "verified_secrets": 1,
      "unknown_secrets": 1,
      "unverified_secrets": 1,
      "scan_duration_seconds": 12.4
    },
    "findings": [
      {
        "repository": "web-app",
        "detector": "AWS",
        "verified": true,
        "verification": "verified",
        "file": ".env"
      }
    ],
    "secrets_by_detector": {
      "GitHub": 2,
      "AWS": 1
    },
    "failed_repos": []
  },
  "hygiene": {
    "clean_repos": 3,
    "repos_with_violations": 2,
    "total_violations": 4,
    "error_repos": 0,
    "failed_repos": [],
    "violation_repos": [
      {
        "repository": "web-app",
        "path": "./web-app",
        "violations": [
          {
            "file_path": ".env",
            "violation_type": "gitignore_violation",
            "size_bytes": null
          }
        ]
      }
    ]
  },
  "fixes": null,
  "message": "✅ Completed in 12.4s • 5 repos • 1 VERIFIED secrets found"
}
```

### Target Specific Repos

```bash
repos audit --repos my-app,web-app
repos audit --fix-gitignore --repos my-app
```

Repository names are exact. If any requested name is absent, the command lists
the missing names and exits before scanning or fixing a partial selection.

---

## Security Best Practices

### 1. Regular Scanning

Run audits regularly in development:
```bash
# Daily/weekly
repos audit --verify
```

### 2. CI/CD Integration

Fail builds on verified or verification-unknown findings and incomplete scans:
```yaml
# GitHub Actions example
- name: Security Audit
  run: repos audit --verify --install-tools
```

Unverified findings remain visible in the report but do not by themselves make
verification mode fail.

### 3. Pre-Push Hooks

Prevent pushing secrets:
```bash
# .git/hooks/pre-push
#!/bin/bash
repos audit --verify || exit 1
```

### 4. Secret Rotation

If secrets are found:
1. **Rotate immediately** - assume compromised
2. Remove from history: `repos audit --fix-secrets`
3. Inspect the rewritten history and preserve the reported recovery bundle
4. Coordinate publication of every affected branch and tag
5. Clean host-specific refs/caches as needed and tell collaborators to re-clone

### 5. Large File Prevention

Prevent large files from being committed:
- Use Git LFS for binary assets
- Add size limits to pre-commit hooks
- Keep repositories lean (<100MB ideal)

### 6. .gitignore Hygiene

Maintain comprehensive `.gitignore`:
```bash
# Fix violations proactively
repos audit --fix-gitignore

# Review before committing
git diff .gitignore
```

---

## Examples

### Full Security Audit

```bash
# 1. Install tools and scan
repos audit --install-tools --verify

# 2. Fix gitignore issues safely
repos audit --fix-gitignore

# 3. Preview destructive fixes
repos audit --fix-large --fix-secrets --dry-run

# 4. Apply all fixes with confirmation
repos audit --fix-all

# 5. Inspect each rewritten repository and its recovery bundle, then coordinate
#    publication of every affected branch and tag
```

### Targeted Cleanup

```bash
# Fix specific repo
repos audit --repos my-app --fix-all

# Only remove large files
repos audit --fix-large

# Only fix secrets
repos audit --fix-secrets --repos api-server
```

### Dry Run Everything

```bash
repos audit --fix-all --dry-run --json > audit-report.json
```

---

**Related Documentation:**
- [Documentation Index](../README.md)
- [Getting Started](../getting_started.md)
- [Commands Reference](commands.md)
- [Troubleshooting](troubleshooting.md)
