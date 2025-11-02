# Getting Started

Fast Git repository management tool written in Rust for batch operations across multiple repositories.

## Installation

```bash
./install.sh  # See installation.md for alternatives
```

## Quick Tour

Run `repos` in a directory to discover and operate on all Git repositories:

```bash
repos
```

Output:
```
📦 Discovered 5 repositories
  my-app
  my-lib
  frontend
  backend
  docs
```

### Push All Repos

```bash
repos push
```

Output:
```
✅ 3 repos pushed  ⚠️  2 already up-to-date
```

### Stage Files Across Repos

```bash
repos stage "*.md"      # Stage markdown files
repos stage "README.md" # Stage specific file
```

### Commit Changes

```bash
repos commit "Update docs"
```

Output:
```
✅ 3 repos committed  ⚠️  2 skipped (no staged changes)
```

### Sync Git Config

```bash
repos config --from-global              # Copy from global config
repos config --name "Alice" --email "alice@example.com"
```

## Common Workflows

### Bulk Updates

Stage, commit, and push across all repos:

```bash
repos stage "*"
repos commit "Bulk update"
repos push
```

### Release Workflow

Publish packages with git tags:

```bash
repos publish --dry-run    # Preview first
repos publish --tag        # Publish + create tags (v1.2.3)
```

### Config Sync

Set name/email across all repos:

```bash
repos config --from-global --force    # No prompts
```

## Next Steps

- [commands.md](guides/commands.md) - Full command reference
- [publishing.md](guides/publishing.md) - Package publishing guide
- [subrepo_management.md](guides/subrepo_management.md) - Advanced nested repo features
