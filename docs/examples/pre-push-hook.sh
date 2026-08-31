#!/bin/bash
# Git Pre-Push Hook: Security Audit
#
# Prevents pushing when secrets verify as active, verification is inconclusive,
# or any repository scan is incomplete.
#
# Installation:
#   1. Save this file as .git/hooks/pre-push in your repository
#   2. Make it executable: chmod +x .git/hooks/pre-push
#   3. Test: git push (should run audit before pushing)

echo "🔒 Running security audit before push..."

# Run the audit directly so its exit status cannot be hidden by a logging pipe.
if ! repos audit --verify --install-tools; then
    echo ""
    echo "❌ PUSH BLOCKED: Security audit found a blocker or could not complete."
    echo ""
    echo "Action required:"
    echo "  1. Review the findings or scanner error above"
    echo "  2. Rotate any exposed credential before changing history"
    echo "  3. Preview cleanup from a clean, synced repo: repos audit --fix-secrets --dry-run"
    echo "  4. Preserve the reported recovery bundle and inspect post-checks"
    echo "  5. Coordinate any required force-push with collaborators"
    echo ""
    echo "To bypass this check (NOT recommended):"
    echo "  git push --no-verify"
    echo ""
    exit 1
fi

echo "✅ No verified or verification-unknown secrets found; audit completed."
exit 0
