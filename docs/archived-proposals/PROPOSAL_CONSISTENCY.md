# PROPOSAL: Project Consistency & Standardization

## Future Command: `sync-repos standardize`

A separate command for ensuring consistent tooling, formatting, and configuration across all repositories of the same type.

## Problem Statement

When managing multiple repositories, you often want:
- Same linting rules across all JavaScript projects
- Same formatter config across all Rust projects
- Consistent CI/CD setup
- Standard `.editorconfig` files
- Consistent package.json scripts

## Proposed Solution

```bash
# Detect project types and suggest standardization
sync-repos standardize

# Apply preset configurations
sync-repos standardize --preset javascript
sync-repos standardize --preset rust
sync-repos standardize --preset python

# Custom templates
sync-repos standardize --template ./my-template
```

### JavaScript Preset Example

Would add/update:
- `.eslintrc.json` - Linting rules
- `.prettierrc` - Formatting rules
- `.editorconfig` - Editor settings
- Standard npm scripts in `package.json`:
  - `npm run lint`
  - `npm run format`
  - `npm run test`
  - `npm run build`
- `.nvmrc` - Node version
- `tsconfig.json` - If TypeScript detected

### Rust Preset Example

Would add/update:
- `rustfmt.toml` - Formatting config
- `.cargo/config.toml` - Cargo settings
- Standard cargo aliases
- Clippy lint settings
- GitHub Actions for Rust

### Python Preset Example

Would add/update:
- `pyproject.toml` - Modern Python config
- `.flake8` or `ruff.toml` - Linting
- `black` configuration - Formatting
- `.python-version` - Python version
- `requirements-dev.txt` - Dev dependencies

## Interactive Mode

```
$ sync-repos standardize

🎨 Project Standardization Analysis

Found project types:
  - 12 JavaScript/TypeScript projects
  - 5 Rust projects
  - 3 Python projects
  - 9 Unknown/Mixed

JavaScript projects missing standard configs:
  ❌ goobits-weather    - Missing: .eslintrc, .prettierrc
  ❌ claude-keeper      - Missing: .prettierrc, .nvmrc
  ❌ codeflow          - Missing: standard npm scripts
  ✓ goobits-store     - Fully configured

Apply JavaScript standards? [y/N]: y
```

## Key Principles

1. **Non-destructive** - Never overwrites custom configurations without asking
2. **Detects existing tools** - Respects if project uses Yarn vs npm, ESLint vs JSHint
3. **Version aware** - Checks package.json to use appropriate configs
4. **Customizable** - Users can define their own templates/presets

## Configuration File

Users could define their own standards in `~/.sync-repos/standards.toml`:

```toml
[javascript]
eslint_extends = ["airbnb", "prettier"]
prettier_config = { semi = false, singleQuote = true }
required_scripts = ["lint", "test", "build"]
node_version = "20.0.0"

[rust]
rust_version = "1.75.0"
clippy_level = "pedantic"

[python]
formatter = "black"
linter = "ruff"
python_version = "3.12"
```

## Benefits

- Ensures consistency across all projects
- Makes it easier to contribute across different repos
- Standardizes CI/CD expectations
- Reduces configuration drift over time

## Implementation Priority

This would be a **separate feature** from `sync-repos clean`, focusing on development experience rather than git hygiene.

Priority: **Low** - Focus on git hygiene first (`clean` command)