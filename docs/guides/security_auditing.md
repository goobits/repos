# Security Auditing and Hygiene

Comprehensive security scanning and repository hygiene checking with automated fixes.

## Overview

The `repos audit` command combines TruffleHog secret scanning with repository hygiene checking to identify security issues and improperly committed files across all repositories.

**Concurrency:**
- TruffleHog scanning: 1 concurrent (CPU-intensive)
- Hygiene checking: 3 concurrent

```bash
repos audit                    # Scan all repos
repos audit --install-tools    # Auto-install TruffleHog
repos audit --verify           # Verify secrets are active
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
brew install trufflesecurity/trufflehog/trufflehog                    # macOS
curl -sSfL https://raw.githubusercontent.com/.../install.sh | sh      # Linux
```

### Secret Detection

Scans git history for exposed credentials and API keys:

```bash
repos audit                    # Detect unverified secrets
repos audit --verify           # Verify if secrets are active (exits 1 on findings)
```

**Verification mode** (`--verify`):
- Tests if secrets are currently active
- **Exits with code 1** if verified secrets found
- Use in CI/CD pipelines to fail builds
- Slower due to API verification calls

### Output

```
🟢 my-app      no secrets
🟡 api-server  3 secrets (unverified)
🔴 web-app     2 secrets (1 verified)

═══════════════════════════════════════════════════════════════════
🔍 SECRET SCANNING RESULTS
═══════════════════════════════════════════════════════════════════
🔴 VERIFIED SECRETS FOUND (1)
   These secrets are confirmed to be active and should be rotated immediately!

📊 SECRETS BY TYPE
   2 × GitHub
   1 × AWS
   1 × Slack

═══════════════════════════════════════════════════════════════════
```

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

Files exceeding **1MB threshold** in git history:

```bash
# Shows top 10 largest files
git rev-list --objects --all | git cat-file --batch-check
```

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

#### Moderate: Untrack Files

```bash
repos audit --fix-all
```

Adds patterns and untracks files:
- Updates `.gitignore`
- Runs `git rm --cached -r` on violating files
- Files remain in working directory
- Reversible with `git add`

#### Destructive: History Rewriting

```bash
# Preview first
repos audit --fix-large --fix-secrets --dry-run

# Apply with confirmation
repos audit --fix-large --fix-secrets
```

**Requirements:**
- `git-filter-repo` must be installed
- Repository must be clean (no uncommitted changes)
- All collaborators must re-clone after push

**What happens:**
1. Creates backup refs: `refs/original/pre-fix-backup-<type>-<timestamp>`
2. Rewrites git history to remove files/secrets
3. Runs aggressive garbage collection
4. Requires `git push --force-with-lease`

**Rollback:**
```bash
git reset --hard refs/original/pre-fix-backup-large-20241101-143022
```

### Interactive Mode

```bash
repos audit --interactive
```

Prompts for each fix type:
```
📋 Fix Summary

Found violations in 3 repositories:
  📝 8 files need .gitignore entries
     → Will only add to .gitignore (files remain tracked)
  📦 2 large files in history
     → Will remove from Git history (requires force-push)

═══════════════════════════════════════════════════════════════════
⚠️  CONFIRMATION REQUIRED
═══════════════════════════════════════════════════════════════════

🔴 DESTRUCTIVE OPERATION - HISTORY REWRITE
   • Git history will be permanently rewritten
   • Backups saved in refs/original/pre-fix-backup-*
   • You will need to force-push: git push --force-with-lease
   • All collaborators must re-clone or reset their branches

   ROLLBACK: git reset --hard refs/original/pre-fix-backup-<timestamp>

═══════════════════════════════════════════════════════════════════

Type 'yes' to proceed or anything else to cancel:
```

---

## Additional Flags

| Flag | Description |
|------|-------------|
| `--json` | Output results in JSON format |
| `--repos <repo1,repo2>` | Target specific repositories (comma-separated) |

### JSON Output

```bash
repos audit --json
```

```json
{
  "truffle": {
    "summary": {
      "total_repos_scanned": 5,
      "repos_with_secrets": 2,
      "total_secrets": 3,
      "verified_secrets": 1,
      "unverified_secrets": 2,
      "scan_duration_seconds": 12.4
    },
    "secrets_by_detector": {
      "GitHub": 2,
      "AWS": 1
    }
  }
}
```

### Target Specific Repos

```bash
repos audit --repos my-app,web-app
repos audit --fix-gitignore --repos my-app
```

---

## Security Best Practices

### 1. Regular Scanning

Run audits regularly in development:
```bash
# Daily/weekly
repos audit --verify
```

### 2. CI/CD Integration

Fail builds on verified secrets:
```yaml
# GitHub Actions example
- name: Security Audit
  run: repos audit --verify --install-tools
```

Exit code 1 if verified secrets found.

### 3. Pre-Push Hooks

Prevent committing secrets:
```bash
# .git/hooks/pre-push
#!/bin/bash
repos audit --verify || exit 1
```

### 4. Secret Rotation

If secrets are found:
1. **Rotate immediately** - assume compromised
2. Remove from history: `repos audit --fix-secrets`
3. Force-push: `git push --force-with-lease`
4. Notify team to re-clone

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

# 5. Push changes
cd ~/repos/my-app && git push --force-with-lease
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
