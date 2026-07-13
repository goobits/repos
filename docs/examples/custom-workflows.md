# Custom Workflows

Use `repos` for the fleet operation and normal Git commands for repository-local
changes. Do not parse human-formatted `repos status` output as a machine API.

## Explicit Repository List

```bash
#!/usr/bin/env bash
set -euo pipefail

branch="feat/update-license"
repositories=("service-api" "service-web")

repos sync

for repository in "${repositories[@]}"; do
  (
    cd "$repository"
    git switch -c "$branch"
    cp ../LICENSE LICENSE
    git add LICENSE
    git commit -m "Update LICENSE"
  )
done

repos push --auto-upstream
```

Use explicit paths from trusted configuration when the workflow will mutate
repositories. This avoids coupling automation to display formatting or partial
name matches.

## CI Publish

```yaml
jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo install goobits-repos
      - run: repos publish --all --dry-run
      - run: repos publish --all
```

Configure npm, Cargo, or PyPI credentials through their standard environment or
credential files before the publish steps. See the
[credentials guide](../guides/credentials_setup.md).
