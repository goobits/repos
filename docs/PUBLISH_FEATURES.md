# Publishing Guide

## Basic Usage

```bash
repos publish                   # Public repos only (default)
repos publish my-app my-lib     # Specific repos
repos publish --dry-run         # Preview without publishing
```

## Visibility Filtering

```bash
repos publish                   # Public repos only (default - safe)
repos publish --all             # All repos (public + private)
repos publish --public-only     # Explicit public only
repos publish --private-only    # Private repos only
```

**Default behavior:** Only publishes public repositories to prevent accidental publishing of private packages.

## Git Tagging

```bash
repos publish --tag             # Create and push git tags after publish
```

**What it does:**
- Creates git tag (e.g., `v1.2.3`) after successful publish
- Pushes tag to remote (`origin`)
- Useful for tracking releases and changelog generation

**Example output:**
```
🟢 my-app        published           published, tagged & pushed v1.2.3
🟢 my-lib        published           published, tagged & pushed v2.0.1
```

## Safety Features

### Clean State Check (Default)

By default, requires no uncommitted changes before publishing.

```bash
# If you have uncommitted changes:
❌ Cannot publish: 2 repositories have uncommitted changes

Repositories with uncommitted changes:
  • my-app
  • my-lib

Commit your changes first, or use --allow-dirty to publish anyway.
```

**Override (not recommended):**
```bash
repos publish --allow-dirty
```

## Recommended Workflow

```bash
# 1. Commit changes
git add .
git commit -m "Prepare release v1.2.3"

# 2. Preview
repos publish --dry-run

# 3. Publish with tags
repos publish --tag

# Done! 🎉
```

## All Flags

| Flag | Description |
|------|-------------|
| `--dry-run` | Preview without publishing |
| `--tag` | Create and push git tags |
| `--allow-dirty` | Skip clean state check |
| `--all` | Publish all repos (public + private) |
| `--public-only` | Only public repos (default) |
| `--private-only` | Only private repos |

## Examples

```bash
# Preview specific package
repos publish my-app --dry-run

# Publish with tags (recommended for releases)
repos publish --tag

# Publish specific package with tags
repos publish my-app --tag

# Publish all including private repos
repos publish --all

# Force publish despite uncommitted changes (not recommended)
repos publish --allow-dirty
```

## Output Examples

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

## Troubleshooting

**"Cannot publish: repositories have uncommitted changes"**
```bash
git add . && git commit -m "Release v1.2.3"
repos publish --tag
```

**"not authenticated"**
```bash
npm login                        # NPM
cargo login YOUR_TOKEN           # Cargo
# Edit ~/.pypirc                 # Python
```

See [CREDENTIALS_SETUP.md](CREDENTIALS_SETUP.md) for details.

**"tag already exists"**

Normal if you've already tagged. To retag:
```bash
git tag -d v1.2.3
git push origin :refs/tags/v1.2.3
repos publish --tag
```

## How It Works

- Detects package type in each repo (npm, cargo, PyPI)
- Checks repository visibility using `gh` CLI (GitHub only)
- Runs appropriate publish command with your existing credentials
- Creates git tags after successful publish (if `--tag` specified)
- Processes 3 packages concurrently

## Credentials

Uses your existing package manager credentials:
- NPM: `~/.npmrc`
- Cargo: `~/.cargo/credentials.toml`
- Python: `~/.pypirc` or env vars

See [CREDENTIALS_SETUP.md](CREDENTIALS_SETUP.md) for setup.
