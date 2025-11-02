# Publishing Credentials

`repos publish` uses your existing package manager credentials to authenticate with registries. Configure credentials for each package manager you use.

## NPM

```bash
npm login                           # Interactive
# OR
npm config set //registry.npmjs.org/:_authToken npm_YOUR_TOKEN

npm whoami                          # Verify
```

**Tokens:** https://www.npmjs.com/settings/YOUR_USERNAME/tokens

## Cargo

```bash
cargo login YOUR_TOKEN              # Sets ~/.cargo/credentials.toml
```

**Tokens:** https://crates.io/settings/tokens

## Python (PyPI)

```bash
# Edit ~/.pypirc
[distutils]
index-servers = pypi

[pypi]
username = __token__
password = pypi-YOUR_TOKEN
```

**Tokens:** https://pypi.org/manage/account/token/

**Alternative:** Environment variables
```bash
export TWINE_USERNAME=__token__
export TWINE_PASSWORD=pypi-YOUR_TOKEN
```

## Private Registries

| Manager | Configuration |
|---------|--------------|
| **NPM** | `npm config set registry https://your-registry.com`<br>`npm login --registry=https://your-registry.com` |
| **Cargo** | Edit `~/.cargo/config.toml`:<br>`[registries.my-registry]`<br>`index = "https://my-registry.com/git/index"` |
| **Python** | Edit `~/.pypirc` with custom repository URL |

## Testing

```bash
npm publish --dry-run                      # NPM
cargo publish --dry-run                    # Cargo
python -m build && twine check dist/*      # Python
```

## Security

```bash
# Protect credential files
chmod 600 ~/.npmrc ~/.cargo/credentials.toml ~/.pypirc
```

**Enable 2FA:**
- NPM: https://www.npmjs.com (Account Settings → 2FA)
- Cargo: https://crates.io/settings/profile
- PyPI: https://pypi.org/manage/account/

**Best practices:** Use tokens (not passwords); prefer project-scoped tokens.

## Troubleshooting

| Error | Solution |
|-------|----------|
| **"not authenticated"** | NPM: `npm logout && npm login`<br>Cargo: `cargo login YOUR_NEW_TOKEN`<br>Python: Verify `~/.pypirc` format |
| **"permission denied"** | NPM: `npm owner add USERNAME PACKAGE`<br>Cargo/PyPI: Add as maintainer in package settings |
| **Multi-account** | Create per-project `.npmrc` or `.cargo/config.toml` |

---

**Related Documentation:**
- [Documentation Index](../README.md)
- [Publishing Guide](publishing.md)
- [Troubleshooting](troubleshooting.md)
