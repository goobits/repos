# PROPOSAL: Git Hygiene Clean Command

## Command: `sync-repos clean`

Focus: **Git repository hygiene only** - fixing git history and tracking issues.

## Core Operations

### 1. Fix .gitignore (`--fix-gitignore`)

**What it does:**
- Detects files that match .gitignore patterns but are tracked
- Detects files matching universal bad patterns (node_modules/, .env, etc.)
- Adds appropriate entries to .gitignore
- Optionally untracks files (with confirmation)

**Safe mode (included in --auto):**
- Only adds entries to .gitignore
- Creates a single commit with .gitignore updates

**Full mode:**
- Adds to .gitignore
- Runs `git rm --cached` on matched files
- Creates cleanup commit

### 2. Fix Large Files (`--fix-large`)

**What it does:**
- Finds files >1MB in git history (configurable threshold)
- Uses BFG Repo-Cleaner to remove from history
- Optionally migrates to Git LFS

**Safety:**
- NEVER included in --auto
- Always creates backup branch
- Shows size impact (before: 500MB → after: 50MB)
- Requires explicit force-push confirmation

### 3. Fix Secrets (`--fix-secrets`)

**What it does:**
- Uses existing TruffleHog detection from audit
- Removes secrets from entire git history
- Provides detailed report of what was removed

**Safety:**
- NEVER included in --auto
- Creates backup branch
- Shows exactly which commits will be rewritten
- Requires manual review of each secret type

## Command Examples

```bash
# Interactive mode - guides through available fixes
sync-repos clean

# Safe auto-fix (only adds to .gitignore)
sync-repos clean --auto

# Specific fixes
sync-repos clean --fix-gitignore           # Interactive gitignore fixes
sync-repos clean --fix-gitignore --untrack # Also untracks files
sync-repos clean --fix-large               # Remove large files from history
sync-repos clean --fix-secrets             # Remove secrets from history

# Combine operations
sync-repos clean --fix-gitignore --fix-large

# Dry run mode
sync-repos clean --dry-run

# Target specific repos
sync-repos clean --repos goobits-weather,claude-keeper
```

## Interactive Flow

```
$ sync-repos clean

🧹 Git Repository Cleanup

Analyzing 29 repositories...

Summary:
  📝 Files needing .gitignore: 27,782 across 17 repos
  🔑 Secrets in history: 727 across 8 repos
  📦 Large files in history: 8 files across 3 repos

Choose action:
  1) Safe: Update .gitignore files only
  2) Moderate: Update .gitignore + untrack files
  3) Full: All fixes including history rewriting
  4) Select specific repositories
  5) Exit

Choice: 2

This will affect 17 repositories:
  goobits-weather: 27,778 files to untrack
  codeflow: 1 file to untrack
  [... list all ...]

Proceed? [y/N]: y

Processing goobits-weather...
  ✓ Added node_modules/ to .gitignore
  ✓ Added dist/ to .gitignore
  ✓ Untracked 27,778 files
  ✓ Created commit: "chore: Update .gitignore and untrack ignored files"

[... continues for each repo ...]

✅ Cleanup complete: 17 repos cleaned, 27,782 files untracked
```

## Implementation Details

### Priority Order
1. **First:** Implement `--fix-gitignore` (most common issue)
2. **Second:** Implement `--fix-large` (using BFG)
3. **Third:** Implement `--fix-secrets` (requires most care)

### Gitignore Intelligence

Group similar patterns for cleaner .gitignore:
```
# Instead of 1000 individual entries:
node_modules/
dist/
*.log
*.tmp

# Not:
# node_modules/package1/
# node_modules/package2/
# ... etc
```

### Large File Handling

```rust
struct LargeFileInfo {
    path: String,
    size: u64,
    first_commit: String,
    impact: String, // "In 45 commits"
}

// Show before cleanup:
"app.zip (45MB) - appears in 12 commits, removing saves 540MB"
```

### Secret Handling

```rust
enum SecretAction {
    Remove,        // Remove file entirely
    Redact,        // Replace secret with REDACTED
    Skip,          // Keep as-is
    ReviewLater,   // Mark for manual review
}
```

## Success Metrics

- Reduces repository sizes significantly (goobits-weather: 500MB → 10MB)
- Eliminates accidental secret exposure
- Prevents CI/CD from processing unnecessary files
- Improves clone/fetch performance

## NOT in Scope

These belong in `sync-repos standardize` (see PROPOSAL_CONSISTENCY.md):
- Code formatting (prettier, rustfmt)
- Linting configuration
- Editor settings
- Development tool setup
- Style consistency

## Next Steps

1. Implement basic `--fix-gitignore` with safe mode
2. Add interactive selection for repos and files
3. Integrate BFG Repo-Cleaner for `--fix-large`
4. Add `--fix-secrets` with careful safety checks
5. Create comprehensive tests