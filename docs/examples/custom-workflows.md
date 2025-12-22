# Custom Workflows with repos

You can leverage `repos` in your own shell scripts to automate complex workflows across multiple repositories.

## Example: Mass branch creation and PR opening

This script creates a new branch in all repositories matching a pattern, makes a change, and pushes.

```bash
#!/bin/bash

# Configuration
BRANCH_NAME="feat/update-license"
REPOS_PATTERN="goobits-"

# 1. Pull latest changes
repos pull --rebase

# 2. Iterate over discovered repos
# Use repos status to get a list of repos (or find them yourself)
for repo in $(repos status | grep "$REPOS_PATTERN" | awk '{print $2}'); do
    echo "Processing $repo..."
    cd "$repo"
    
    # Create branch
    git checkout -b "$BRANCH_NAME"
    
    # Apply change
    cp ../LICENSE ./LICENSE
    
    # Stage and commit
    repos stage LICENSE
    repos commit "Update LICENSE file"
    
    cd ..
done

# 3. Push all new branches
repos push --force
```

## Example: CI/CD integration for massive monorepos

If you have a large monorepo with many independent packages, you can use `repos publish` to only publish what changed.

```yaml
jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install repos
        run: curl -sSf https://raw.githubusercontent.com/goobits/repos/main/install.sh | sh
      - name: Publish all public packages
        run: repos publish --public-only --tag
```
