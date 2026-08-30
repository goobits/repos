<h1 align="center">repos</h1>

<p align="center"><strong>Inspect and coordinate Git work across a fleet of repositories.</strong></p>
<p align="center">See what needs attention, save bounded changes, synchronize safe remote work, and keep nested-repository drift visible.</p>

<p align="center">
  <a href="#why-repos">Why repos</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="#daily-workflow">Daily workflow</a> ·
  <a href="#safety-model">Safety model</a> ·
  <a href="#documentation">Documentation</a>
</p>

---

## Why repos

A workspace with many Git repositories makes ordinary status, commit, pull,
and push work easy to miss or perform in the wrong place. `repos` discovers the
fleet, reports actionable state, and applies explicit Git workflows across the
selected repositories.

It does not turn every worktree into one repository. Each repository keeps its
own branch, remote, upstream, ignore rules, and commit history.

## Quick start

Requires Rust and Cargo 1.78 or newer. Build from this checkout while keeping
Cargo output outside the source tree:

```bash
export CARGO_TARGET_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/goobits-repos/target"
cargo build --locked --release
"$CARGO_TARGET_DIR/release/repos" --help
```

[`install.sh`](install.sh) builds and installs the binary into a user or system
binary directory. Review it before use: when Rust is absent it can offer to
install Rust and update shell configuration.

## Daily workflow

```bash
repos status --needs-work
repos save "describe the change" --dry-run
repos save "describe the change"
repos sync
```

The CLI also exposes focused `fetch`, `pull`, `push`, `stage`, `unstage`,
`commit`, `config`, `audit`, `doctor`, `publish`, and subrepository workflows.
Use `repos <command> --help` for exact options.

## Safety model

Status and dry-run paths are the preview surfaces. `save` stages tracked files
by default; `--include-untracked` and `--all` explicitly widen that scope. It
then commits and pushes according to the selected mode. `sync` pulls safe remote
changes, pushes local commits, and reports nested drift. Missing remotes,
upstreams, conflicts, dirty state, and failed inspection remain visible rather
than being silently repaired.

Repository discovery does not authorize mutation. Review the selected targets
and dry-run output before fleet-wide commands, especially when nested
repositories or submodules are present.

## Documentation

- [Documentation index](docs/README.md)
- [Getting started](docs/getting_started.md)
- [Installation](docs/installation.md)
- [Command guide](docs/guides/commands.md)
- [Architecture](docs/architecture.md)
- [Subrepository management](docs/guides/subrepo_management.md)
- [Troubleshooting](docs/guides/troubleshooting.md)

## Development

Use `cargo fmt`, `cargo clippy`, and the repository's Rust test commands as
documented in [AGENTS.md](AGENTS.md). Keep generated Cargo state outside the
checkout.

## License

[MIT](LICENSE) © repos contributors.
