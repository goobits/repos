# Publishing

Publish packages to npm, Cargo, or PyPI with git tag support.

## Quick Start

```bash
repos publish                   # Public repos only (safe default)
repos publish --dry-run         # Preview first
repos publish --tag             # Publish + create git tags
repos publish my-app my-lib     # Specific repos
```

## Recommended Workflow

```bash
git commit -m "Release v1.2.3"  # 1. Commit changes
repos publish --dry-run         # 2. Preview
repos publish --tag             # 3. Publish + tag
```

## Flags

| Flag | Description |
|------|-------------|
| `--dry-run` | Preview without publishing |
| `--tag` | Create and push git tags (e.g., `v1.2.3`) |
| `--allow-dirty` | Skip clean state check (not recommended) |
| `--all` | Publish all repos (public + private) |
| `--public-only` | Only public repos (default) |
| `--private-only` | Only private repos |

**Safety:** By default, requires clean working directory and only publishes public repos.

## Example Output

```
📦 Publishing 5 packages

🟢 my-app        published           published, tagged & pushed v1.2.3
🟢 my-lib        published           published, tagged & pushed v2.0.1
🟠 my-cli        already-published   already published
🔴 broken-pkg    failed              not authenticated (run: npm login)
🟢 my-util       published           published, tagged & pushed v1.1.0

✅ 3 published  ⚠️  1 already published  ❌ 1 failed
```

**Status indicators:** 🟢 success | 🟠 skipped | 🔴 failed

## How It Works

- Auto-detects package type (npm/Cargo/PyPI) per repo
- Checks visibility via `gh` CLI (GitHub only; defaults to public otherwise)
- Uses existing credentials (`~/.npmrc`, `~/.cargo/credentials.toml`, `~/.pypirc`)
- Creates git tags after successful publish (if `--tag`)
- Processes 3 packages concurrently

See [credentials_setup.md](credentials_setup.md) for credential configuration.

## Troubleshooting

| Error | Solution |
|-------|----------|
| **"uncommitted changes"** | Commit first: `git add . && git commit -m "Release v1.2.3"` |
| **"not authenticated"** | See [credentials_setup.md](credentials_setup.md) |
| **"tag already exists"** | Delete tag: `git tag -d v1.2.3 && git push origin :refs/tags/v1.2.3` |
