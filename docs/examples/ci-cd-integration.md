# CI/CD Integration

`repos` is designed to be CI-friendly. It can be used to audit, sync, and publish multiple packages in automated pipelines.

## GitHub Actions

### Basic Security Audit

```yaml
name: Security Audit
on: [push, pull_request]

jobs:
  audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0 # Important for history scanning
      - name: Install repos
        run: curl -sSf https://raw.githubusercontent.com/goobits/repos/main/install.sh | sh
      - name: Run Audit
        run: repos audit --verify
```

### Automated Publishing

```yaml
name: Publish
on:
  push:
    tags:
      - 'v*'

jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          registry-url: 'https://registry.npmjs.org'
      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
      - name: Install repos
        run: curl -sSf https://raw.githubusercontent.com/goobits/repos/main/install.sh | sh
      - name: Publish all packages
        run: repos publish --all --tag
        env:
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_TOKEN }}
```

## GitLab CI

```yaml
audit:
  image: rust:latest
  script:
    - curl -sSf https://raw.githubusercontent.com/goobits/repos/main/install.sh | sh
    - repos audit --verify
```
