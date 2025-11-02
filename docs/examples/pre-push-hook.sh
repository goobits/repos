#!/bin/bash
# Git Pre-Push Hook: Security Audit
#
# Prevents pushing if verified secrets are found in repository history.
#
# Installation:
#   1. Save this file as .git/hooks/pre-push in your repository
#   2. Make it executable: chmod +x .git/hooks/pre-push
#   3. Test: git push (should run audit before pushing)

echo "🔒 Running security audit before push..."

# Run repos audit with verification
if ! repos audit --verify --install-tools 2>&1 | tee /tmp/repos-audit.log; then
    echo ""
    echo "❌ PUSH BLOCKED: Verified secrets found in repository!"
    echo ""
    echo "Action required:"
    echo "  1. Review the secrets found above"
    echo "  2. Rotate compromised credentials immediately"
    echo "  3. Remove secrets from history: repos audit --fix-secrets"
    echo "  4. Force push: git push --force-with-lease"
    echo ""
    echo "To bypass this check (NOT recommended):"
    echo "  git push --no-verify"
    echo ""
    exit 1
fi

echo "✅ No verified secrets found. Proceeding with push..."
exit 0
