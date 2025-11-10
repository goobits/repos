# Examples

Practical templates for integrating `repos` into your workflows. Copy these templates as starting points for CI/CD pipelines, git hooks, and automation scripts.

## CI/CD Integration

### GitHub Actions

**[github-actions-security-audit.yml](github-actions-security-audit.yml)**
Security audit workflow that:
- Runs on every push and pull request
- Scans for secrets with verification
- Fails build if verified secrets found
- Runs weekly on schedule
- Uploads audit reports as artifacts
- Comments on PRs with findings

**[github-actions-publish.yml](github-actions-publish.yml)**
Automated publishing workflow that:
- Triggers on version tags (v1.2.3)
- Publishes to npm, Cargo, and PyPI
- Supports multi-package repositories
- Creates GitHub releases
- Runs dry-run before publishing

### Git Hooks

**[pre-push-hook.sh](pre-push-hook.sh)**
Pre-push hook that:
- Runs security audit before every push
- Blocks push if verified secrets found
- Provides clear remediation steps
- Can be bypassed with `--no-verify` if needed

Installation:
```bash
cp pre-push-hook.sh .git/hooks/pre-push
chmod +x .git/hooks/pre-push
```

## Usage Examples

### Complete Release Workflow

```bash
# 1. Update package versions
# Edit package.json, Cargo.toml, pyproject.toml, etc.

# 2. Commit version bumps
repos stage "package.json" "Cargo.toml" "pyproject.toml"
repos commit "chore: Bump version to v1.2.3"

# 3. Run security audit
repos audit --verify

# 4. Test publishing (dry run)
repos publish --dry-run

# 5. Publish packages and create tags
repos publish --tag --all

# 6. Push changes and tags
repos push
git push --tags
```

### Security Audit Workflow

```bash
# 1. Install TruffleHog and scan
repos audit --install-tools --verify

# 2. Fix gitignore issues (safe)
repos audit --fix-gitignore

# 3. Preview destructive fixes
repos audit --fix-large --fix-secrets --dry-run

# 4. Apply fixes interactively
repos audit --interactive

# 5. Or apply all fixes automatically
repos audit --fix-all

# 6. Push updated history
cd ~/repos/my-repo
git push --force-with-lease
```

### Subrepo Synchronization

```bash
# 1. Detect drift
repos subrepo status

# 2. Review full status (including synced repos)
repos subrepo status --all

# 3. Sync to specific commit (safe, with stash)
repos subrepo sync shared-lib --to abc1234 --stash

# 4. Update to latest from remote
repos subrepo update shared-lib

# 5. Verify synchronization
repos subrepo status
```

### Config Synchronization

```bash
# 1. Check current configs
git config --list

# 2. Preview changes
repos config --from-global --dry-run

# 3. Sync from global config
repos config --from-global

# 4. Or set specific values
repos config --name "Alice Developer" --email "alice@example.com"

# 5. Force sync without prompts
repos config --from-global --force
```

## Shell Scripts

### Bulk Repository Update Script

```bash
#!/bin/bash
# update-all-repos.sh - Update and sync all repositories

set -e

echo "📦 Discovering repositories..."
repos status

echo ""
echo "🔄 Pulling latest changes..."
# Note: repos doesn't have a pull command yet, so use git directly
for repo in */; do
    if [ -d "$repo/.git" ]; then
        echo "  Pulling $repo"
        (cd "$repo" && git pull)
    fi
done

echo ""
echo "⚙️  Syncing git config..."
repos config --from-global --force

echo ""
echo "🔒 Running security audit..."
repos audit --verify

echo ""
echo "✅ All repositories updated and validated!"
```

### Release Preparation Script

```bash
#!/bin/bash
# prepare-release.sh - Prepare multi-repo release

VERSION=$1

if [ -z "$VERSION" ]; then
    echo "Usage: $0 <version>"
    echo "Example: $0 1.2.3"
    exit 1
fi

set -e

echo "🏷️  Preparing release v$VERSION"
echo ""

# Update version in all package files
echo "📝 Updating package versions..."
find . -name "package.json" -exec sed -i "s/\"version\": \".*\"/\"version\": \"$VERSION\"/" {} \;
find . -name "Cargo.toml" -exec sed -i "s/^version = \".*\"/version = \"$VERSION\"/" {} \;
find . -name "pyproject.toml" -exec sed -i "s/^version = \".*\"/version = \"$VERSION\"/" {} \;

echo "📝 Staging changes..."
repos stage "package.json" "Cargo.toml" "pyproject.toml"

echo "💾 Committing version bump..."
repos commit "chore: Bump version to v$VERSION"

echo "🔒 Running security audit..."
repos audit --verify

echo "🧪 Testing publish (dry run)..."
repos publish --dry-run

echo ""
echo "✅ Release v$VERSION prepared!"
echo ""
echo "Next steps:"
echo "  1. Review changes: git log -1"
echo "  2. Publish: repos publish --tag --all"
echo "  3. Push: repos push && git push --tags"
```

## JSON Output Processing

### Parse Audit Results with jq

```bash
# Run audit and save JSON
repos audit --json > audit-report.json

# Count total secrets
jq '.truffle.summary.total_secrets' audit-report.json

# List secrets by type
jq '.truffle.secrets_by_detector' audit-report.json

# Get repos with secrets
jq '.truffle.repos_with_secrets' audit-report.json

# Filter only verified secrets
jq 'select(.truffle.summary.verified_secrets > 0)' audit-report.json
```

### Generate Security Report

```bash
#!/bin/bash
# security-report.sh - Generate weekly security report

DATE=$(date +%Y-%m-%d)
REPORT="security-report-$DATE.md"

echo "# Security Audit Report - $DATE" > $REPORT
echo "" >> $REPORT

# Run audit
repos audit --verify --json > audit-$DATE.json

# Extract summary
echo "## Summary" >> $REPORT
echo "" >> $REPORT
echo "- **Total Repos Scanned:** $(jq '.truffle.summary.total_repos_scanned' audit-$DATE.json)" >> $REPORT
echo "- **Repos with Secrets:** $(jq '.truffle.summary.repos_with_secrets' audit-$DATE.json)" >> $REPORT
echo "- **Total Secrets:** $(jq '.truffle.summary.total_secrets' audit-$DATE.json)" >> $REPORT
echo "- **Verified Secrets:** $(jq '.truffle.summary.verified_secrets' audit-$DATE.json)" >> $REPORT
echo "" >> $REPORT

echo "## Secrets by Type" >> $REPORT
echo "" >> $REPORT
jq -r '.truffle.secrets_by_detector | to_entries[] | "- **\(.key):** \(.value)"' audit-$DATE.json >> $REPORT

echo "" >> $REPORT
echo "Report generated: $DATE" >> $REPORT

cat $REPORT
```

---

**Related Documentation:**
- [Documentation Index](../README.md)
- [Getting Started](../getting_started.md)
- [Security Auditing](../guides/security_auditing.md)
- [Publishing Guide](../guides/publishing.md)
