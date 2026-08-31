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

**Safety:** By default, only public repositories are selected. On an attached
branch, a real publish requires a fully pushed release commit that is not behind
its upstream; the worktree must also be clean unless `--allow-dirty` is used. A
detached release is allowed only when the exact local and remote
`v<manifest-version>` tag both point to `HEAD`. That provenance check applies
with or without `--tag`; the flag controls only tag creation/push afterward.

## Example Output

```
repos publish
✓ Completed in 8.2s

▌ Summary
  ✓ Published         2
  ✓ Already published 1
  ! Failed            1
  · Checked           4

▌ Published
  ✓ my-app                   published v1.2.3
    ↳ path: ./my-app

▌ Failed
  ! broken-pkg              not authenticated
    ↳ path: ./broken-pkg
    ↳ next: inspect registry credentials/version, then retry `repos publish broken-pkg`
```

The final report groups named outcomes, paths, and failure-specific next steps.

## How It Works

- Auto-detects package type (npm/Cargo/PyPI) per repo
- Checks visibility via `gh` CLI (GitHub only; unknown visibility is treated as private)
- Uses existing credentials (`~/.npmrc`, `~/.cargo/credentials.toml`, `~/.pypirc`)
- Rejects any missing requested repository name before package inspection
- Runs clean/release provenance preflight before any real registry mutation
- Reads local manifest dependencies and publishes dependency waves in order
- Parses each static manifest once; malformed manifests and dynamic-only `setup.py` metadata are reported as inspection failures
- Rejects duplicate package identities and dependency cycles before publishing
- Builds Python packages in a private output directory and uploads only the
  exact artifacts created by that invocation, never stale `dist/` files
- Terminates the package-manager command on timeout, including its process group
  on Unix. Because a registry may already have accepted an upload, the result
  says the outcome is unknown and requires a registry check before retrying
- Creates or pushes matching git tags after a successful or already-published registry result (if `--tag`)
- Processes up to 8 independent packages concurrently

Learn more about [credential configuration](credentials_setup.md).

## Troubleshooting

| Error | Solution |
|-------|----------|
| **"uncommitted changes"** | Commit first: `repos save "Release v1.2.3"` or stage explicitly |
| **"not authenticated"** | Configure [publishing credentials](credentials_setup.md) |
| **"tag already exists"** | Inspect `git rev-parse v1.2.3` and `git rev-parse HEAD`. If they differ, bump the package version and publish a new immutable tag. |
| **"release tag ... is not published"** | For detached `HEAD`, publish the exact `v<manifest-version>` tag to the selected remote before retrying. |
| **"requested repositories were not found"** | Use exact discovery names from `repos status`; the command will not publish a partial target list. |
| **"registry outcome is unknown"** | Check the package/version in the registry before retrying; the server may have accepted the timed-out upload. |

---

**Related Documentation:**
- [Documentation Index](../README.md)
- [Credentials Setup](credentials_setup.md)
- [Commands Reference](commands.md)
- [Troubleshooting](troubleshooting.md)
