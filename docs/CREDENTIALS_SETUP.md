# Publishing Credentials Setup

`repos publish` uses your existing package manager credentials.

## NPM

```bash
# Login
npm login

# Or use token
npm config set //registry.npmjs.org/:_authToken YOUR_TOKEN

# Or edit ~/.npmrc
//registry.npmjs.org/:_authToken=npm_YOUR_TOKEN

# Verify
npm whoami
```

Get tokens: https://www.npmjs.com/settings/YOUR_USERNAME/tokens

## Cargo

```bash
# Login
cargo login YOUR_TOKEN

# Or edit ~/.cargo/credentials.toml
[registry]
token = "YOUR_TOKEN"
```

Get tokens: https://crates.io/settings/tokens

## Python (PyPI)

```bash
# Edit ~/.pypirc
[distutils]
index-servers = pypi

[pypi]
username = __token__
password = pypi-YOUR_TOKEN

# Or use environment variables
export TWINE_USERNAME=__token__
export TWINE_PASSWORD=pypi-YOUR_TOKEN

# Verify
pip install twine
twine check dist/*
```

Get tokens: https://pypi.org/manage/account/token/

## Private Registries

### NPM
```bash
npm config set registry https://your-registry.com
npm login --registry=https://your-registry.com
```

### Cargo
Edit `~/.cargo/config.toml`:
```toml
[registries.my-registry]
index = "https://my-registry.com/git/index"
token = "YOUR_TOKEN"
```

### Python
Edit `~/.pypirc`:
```ini
[distutils]
index-servers = company

[company]
repository = https://pypi.company.com
username = your_username
password = your_password
```

## Testing

```bash
# Test each package manager individually
cd your-npm-package && npm publish --dry-run
cd your-cargo-package && cargo publish --dry-run
cd your-python-package && python -m build && twine check dist/*
```

## Security

```bash
# Protect credential files
chmod 600 ~/.npmrc ~/.cargo/credentials.toml ~/.pypirc

# Enable 2FA
# - https://www.npmjs.com (Account Settings → 2FA)
# - https://crates.io/settings/profile
# - https://pypi.org/manage/account/

# Use tokens, not passwords
# Use project-scoped tokens when possible
```

## Troubleshooting

**"not authenticated":**
```bash
npm logout && npm login                    # NPM
cargo login YOUR_NEW_TOKEN                 # Cargo
cat ~/.pypirc                              # Python - check format
```

**"permission denied":**
- NPM: `npm owner add USERNAME PACKAGE`
- Cargo/PyPI: Add as project maintainer

**Multi-account setup:**
Create per-project `.npmrc` or `.cargo/config.toml`
