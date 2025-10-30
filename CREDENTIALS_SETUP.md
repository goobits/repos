# Publishing Credentials Setup

`repos publish` uses your existing package manager credentials. Here's how to configure each one:

---

## 📦 NPM (Node.js packages)

### Option 1: Interactive Login (Recommended)
```bash
npm login
```

This will prompt for:
- Username
- Password
- Email
- 2FA code (if enabled)

Credentials are stored in `~/.npmrc`

### Option 2: Authentication Token
```bash
npm config set //registry.npmjs.org/:_authToken YOUR_TOKEN
```

Get your token from: https://www.npmjs.com/settings/YOUR_USERNAME/tokens

### Option 3: Manual `.npmrc`
Create/edit `~/.npmrc`:
```ini
//registry.npmjs.org/:_authToken=npm_YOUR_TOKEN_HERE
```

### Verify Setup
```bash
npm whoami
# Should show your username
```

---

## 📦 Cargo (Rust packages)

### Login with Token
```bash
cargo login YOUR_CRATES_IO_TOKEN
```

Get your token from: https://crates.io/settings/tokens

This stores the token in `~/.cargo/credentials.toml`

### Manual Setup
Create/edit `~/.cargo/credentials.toml`:
```toml
[registry]
token = "YOUR_CRATES_IO_TOKEN"
```

### Verify Setup
```bash
cargo login --help
# Check if credentials file exists
cat ~/.cargo/credentials.toml
```

---

## 🐍 Python (PyPI packages)

### Option 1: Using `.pypirc` (Recommended)
Create/edit `~/.pypirc`:
```ini
[distutils]
index-servers =
    pypi

[pypi]
username = __token__
password = pypi-YOUR_TOKEN_HERE
```

Get your token from: https://pypi.org/manage/account/token/

### Option 2: Environment Variables
```bash
export TWINE_USERNAME=__token__
export TWINE_PASSWORD=pypi-YOUR_TOKEN_HERE
```

Add to your `~/.bashrc` or `~/.zshrc` to persist.

### Option 3: Keyring (Most Secure)
```bash
pip install keyring
keyring set https://upload.pypi.org/legacy/ __token__
# Enter your token when prompted
```

Then add to `~/.pypirc`:
```ini
[pypi]
username = __token__
```

### Verify Setup
```bash
# Install twine if needed
pip install twine

# Check configuration
twine check dist/*  # (if you have a dist/ directory)
```

---

## 🔐 Private Registries

### NPM Private Registry
```bash
npm config set registry https://your-registry.com
npm login --registry=https://your-registry.com
```

Or in `.npmrc`:
```ini
registry=https://your-registry.com
//your-registry.com/:_authToken=YOUR_TOKEN
```

### Cargo Private Registry
Edit `~/.cargo/config.toml`:
```toml
[registries.my-registry]
index = "https://my-registry.com/git/index"
token = "YOUR_TOKEN"
```

### Python Private Registry
In `~/.pypirc`:
```ini
[distutils]
index-servers =
    company

[company]
repository = https://pypi.company.com
username = your_username
password = your_password
```

Then publish with: `twine upload --repository company dist/*`

---

## 🧪 Testing Credentials

Before running `repos publish`, test each package manager individually:

```bash
# Test NPM
cd your-npm-package
npm publish --dry-run

# Test Cargo
cd your-cargo-package
cargo publish --dry-run

# Test Python
cd your-python-package
python -m build
twine check dist/*
```

Once these work individually, `repos publish` will use the same credentials automatically!

---

## 🔒 Security Best Practices

### 1. Use Tokens, Not Passwords
- NPM: Use automation tokens (not your account password)
- Cargo: Use API tokens
- PyPI: Use project-scoped tokens when possible

### 2. Use 2FA
Enable two-factor authentication on:
- https://www.npmjs.com (Account Settings → Two-Factor Authentication)
- https://crates.io/settings/profile (Account Settings)
- https://pypi.org/manage/account/ (Account Settings → Enable 2FA)

### 3. Protect Credential Files
```bash
chmod 600 ~/.npmrc
chmod 600 ~/.cargo/credentials.toml
chmod 600 ~/.pypirc
```

### 4. Use Environment-Specific Tokens
For CI/CD, use separate tokens with limited permissions:
- NPM: Create "Automation" tokens
- Cargo: Create tokens with publish-only permissions
- PyPI: Create project-scoped tokens

---

## 🚨 Troubleshooting

### "not authenticated" Error

**NPM:**
```bash
npm logout
npm login
```

**Cargo:**
```bash
cargo login YOUR_NEW_TOKEN
```

**Python:**
```bash
# Check .pypirc exists and has correct format
cat ~/.pypirc
# Regenerate token on pypi.org if needed
```

### "permission denied" Error

Check that your account has permission to publish the package:
- NPM: You must be added as a maintainer (`npm owner add USERNAME PACKAGE`)
- Cargo: You must be added as an owner
- PyPI: You must be added as a project maintainer

### Multi-Account Setup

If you need different credentials for different projects:

**NPM (per-project `.npmrc`):**
```bash
cd your-project
echo "//registry.npmjs.org/:_authToken=YOUR_TOKEN" > .npmrc
```

**Cargo (per-project config):**
Create `.cargo/config.toml` in project:
```toml
[registry]
token = "YOUR_TOKEN"
```

**Python (specify at runtime):**
Set environment variables per-project in `.env` file.

---

## 📋 Quick Setup Checklist

- [ ] NPM: Run `npm login` or configure `~/.npmrc`
- [ ] Cargo: Run `cargo login YOUR_TOKEN`
- [ ] Python: Create `~/.pypirc` with token
- [ ] Test: Run `npm whoami`, check cargo credentials, test twine
- [ ] Security: Enable 2FA on all registries
- [ ] Security: Set proper file permissions (600)
- [ ] Ready: Run `repos publish --dry-run` to verify!

---

## 🎯 How `repos publish` Uses These

When you run `repos publish`, the command:

1. Detects package type in each repo
2. Runs the appropriate publish command:
   - NPM: `npm publish` (uses `~/.npmrc`)
   - Cargo: `cargo publish` (uses `~/.cargo/credentials.toml`)
   - Python: `twine upload` (uses `~/.pypirc` or env vars)
3. Each tool automatically uses your configured credentials

**No special repos configuration needed!** If the native tools work, `repos publish` will work.
