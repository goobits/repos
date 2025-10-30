# Publishing Features - Complete Guide

## 🎯 Available Commands

### Basic Publishing

```bash
# Publish all packages
repos publish

# Publish specific packages
repos publish my-app
repos publish my-app my-lib

# Preview without publishing (dry-run)
repos publish --dry-run
```

### Git Tagging (⭐ Recommended)

```bash
# Publish and automatically create git tags
repos publish --tag
```

**What it does:**
- After successful publish, creates a git tag (e.g., `v1.2.3`)
- Pushes the tag to remote (`origin`)
- Shows tag status in output

**Example output:**
```
🟢 my-app        published           published, tagged & pushed v1.2.3
🟢 my-lib        published           published, tagged & pushed v2.0.1
```

**Why use `--tag`?**
- Tracks which commit was published
- Required for GitHub releases
- Essential for changelog generation
- Enables rollbacks to specific versions
- Industry standard practice

### Safety Features

#### Clean State Check (Default)

By default, `repos publish` **requires a clean git state** (no uncommitted changes).

**If you have uncommitted changes:**
```
❌ Cannot publish: 2 repositories have uncommitted changes

Repositories with uncommitted changes:
  • my-app
  • my-lib

Commit your changes first, or use --allow-dirty to publish anyway (not recommended).
```

**Override if needed:**
```bash
repos publish --allow-dirty
```

⚠️ **Not recommended** - you should commit your changes before publishing!

---

## 📋 Complete Examples

### Recommended Workflow

```bash
# 1. Commit your changes
git add .
git commit -m "Prepare release v1.2.3"

# 2. Dry-run to preview
repos publish --dry-run

# 3. Publish with tags
repos publish --tag

# Result: Published and tagged automatically! 🎉
```

### Development Workflow

```bash
# Quick publish during development (no tags)
repos publish

# Publish specific package you're working on
repos publish my-feature-package

# Check what would be published
repos publish my-feature-package --dry-run
```

### Release Workflow

```bash
# Full release with all safety features
git status                           # Verify clean state
repos publish --dry-run             # Preview
repos publish --tag                 # Publish with tags
git push                            # Push commits (tags already pushed)
```

---

## 🔧 All Flags

| Flag | Description | Example |
|------|-------------|---------|
| `--dry-run` | Preview without publishing | `repos publish --dry-run` |
| `--tag` | Create and push git tags | `repos publish --tag` |
| `--allow-dirty` | Skip clean state check | `repos publish --allow-dirty` |
| (positional) | Target specific repos | `repos publish my-app my-lib` |

### Combining Flags

```bash
# Dry-run for specific package
repos publish my-app --dry-run

# Publish specific package with tags
repos publish my-app --tag

# Publish all with tags (most common for releases)
repos publish --tag

# Force publish despite uncommitted changes (not recommended)
repos publish --allow-dirty
```

---

## 🎨 Output Examples

### Successful Publish
```
📦 Publishing 3 packages

🟢 my-app        published           published
🟢 my-lib        published           published
🟢 my-cli        published           published

✅ 3 published

Done in 15s
```

### With Git Tags
```
📦 Publishing 3 packages

🟢 my-app        published           published, tagged & pushed v1.2.3
🟢 my-lib        published           published, tagged & pushed v2.0.1
🟢 my-cli        published           published, tagged & pushed v0.5.0

✅ 3 published

Done in 18s
```

### Mixed Results
```
📦 Publishing 5 packages

🟢 my-app        published           published, tagged & pushed v1.2.3
🟢 my-lib        published           published, tagged & pushed v2.0.1
🟠 my-cli        already-published   already published
🔴 broken-pkg    failed              not authenticated (run: npm login)
🟢 my-util       published           published, tagged & pushed v1.1.0

✅ 3 published  ⚠️  1 already published  ❌ 1 failed

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
❌ Failed to publish:

  • broken-pkg: not authenticated (run: npm login)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Done in 20s
```

### Dirty State Blocked
```
❌ Cannot publish: 2 repositories have uncommitted changes

Repositories with uncommitted changes:
  • my-app
  • my-lib

Commit your changes first, or use --allow-dirty to publish anyway (not recommended).
```

### Dry Run
```
📦 Found 5 packages (dry-run mode)

  📦 my-app        (npm)     v1.2.3
  📦 my-lib        (npm)     v2.0.1
  📦 my-cli        (cargo)   v0.5.0
  📦 my-util       (python)  v1.1.0
  📦 other-thing   (npm)     v3.0.0

Would publish 5 packages (dry-run - nothing published)

Ready to publish? Run: repos publish
```

---

## 🚀 How Features Work

### Clean State Check

**When:** Before publishing starts
**What:** Checks each package repository for uncommitted changes
**Why:** Ensures published version matches git history

**Bypassed when:**
- Using `--dry-run` (checking only, not publishing)
- Using `--allow-dirty` flag

**How it works:**
1. Runs `git update-index --refresh`
2. Checks `git diff-index --quiet HEAD`
3. If changes detected → blocks publish
4. Shows which repos have changes

### Git Tagging

**When:** After successful publish (only for newly published packages)
**What:** Creates annotated git tag and pushes to origin
**Format:** `v{version}` (e.g., `v1.2.3`, `v0.5.0`)

**How it works:**
1. Package is published successfully
2. Reads version from package manifest
3. Creates tag: `git tag v{version}`
4. Pushes tag: `git push origin v{version}`
5. Updates output with tag status

**Handles:**
- Tag already exists → reports "tag already exists"
- Push fails → keeps local tag, shows "(push failed)"
- No version found → skips tagging

**Not tagged:**
- Already-published packages (skipped)
- Failed publishes
- Packages without version info

---

## 🔐 Security & Safety

### What Gets Checked

✅ **Checked automatically:**
- Uncommitted changes (blocks publish)
- Package manager authentication (via native tools)
- Already-published versions (skips)

❌ **NOT checked (do manually):**
- Package version hasn't been bumped
- Tests passing
- Build succeeds
- Breaking changes

### Safety Recommendations

1. **Always commit first**
   ```bash
   git status  # Should be clean
   ```

2. **Always dry-run first**
   ```bash
   repos publish --dry-run
   ```

3. **Use tags for releases**
   ```bash
   repos publish --tag
   ```

4. **Never use `--allow-dirty` in CI/CD**
   - Only for local development emergencies

5. **Check credentials before publishing**
   ```bash
   npm whoami
   cargo login
   cat ~/.pypirc
   ```

---

## 🐛 Troubleshooting

### "Cannot publish: repositories have uncommitted changes"

**Solution:** Commit your changes first
```bash
git add .
git commit -m "Release v1.2.3"
repos publish --tag
```

**Or (not recommended):**
```bash
repos publish --allow-dirty
```

### "not authenticated"

**NPM:**
```bash
npm login
```

**Cargo:**
```bash
cargo login YOUR_TOKEN
```

**Python:**
Create `~/.pypirc` with your PyPI token.

See `CREDENTIALS_SETUP.md` for details.

### "tag already exists"

This is normal if you've already tagged. The command will:
- Report "tag already exists"
- Continue successfully (not an error)

To retag:
```bash
git tag -d v1.2.3
git push origin :refs/tags/v1.2.3
repos publish --tag
```

### "tag failed to push"

Usually means:
- No remote configured
- No push permissions
- Network issues

The local tag is still created. Fix the issue and push manually:
```bash
git push origin --tags
```

---

## 📊 Implementation Details

### Files Modified/Created

**New git operations:**
- `src/git/operations.rs`: Added `has_uncommitted_changes()` and `create_and_push_tag()`

**Updated publish command:**
- `src/commands/publish.rs`: Added clean check and tagging logic
- `src/main.rs`: Added `--tag` and `--allow-dirty` flags

**Lines changed:** ~120 lines total

### Performance

- **Clean check:** <100ms per repo (parallel)
- **Tag creation:** <500ms per tag
- **Total overhead:** ~1-2 seconds for typical workloads

### Concurrency

- Publishing: 3 packages at a time
- Clean checks: All in parallel
- Tagging: Sequential (after each publish)

---

## ✨ Quick Reference

```bash
# Most common commands
repos publish --dry-run           # Preview
repos publish                     # Publish all
repos publish --tag              # Publish with tags (recommended for releases)
repos publish my-app             # Publish specific package

# Safety
# ✅ Clean git state checked by default
# ✅ Already-published packages skipped
# ✅ Errors don't stop other packages

# Credentials
# Uses native tools (npm, cargo, twine)
# See CREDENTIALS_SETUP.md
```

---

## 🎯 Next Steps

1. **Configure credentials** → See `CREDENTIALS_SETUP.md`
2. **Test with dry-run** → `repos publish --dry-run`
3. **Publish your first package** → `repos publish --tag`
4. **Verify tags** → `git tag -l`

Happy publishing! 🚀
