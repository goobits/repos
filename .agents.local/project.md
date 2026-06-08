# Project Policy

Repo-specific facts for this checkout. Keep reusable agent behavior in
`.agents/`; keep local project facts here.

## Project Summary

- Name: repos
- Purpose: Fleet-scale Git orchestration CLI for managing many Git repositories.
- Primary language/framework: Rust CLI using Tokio and Clap.
- Package manager: Cargo.
- Workspace/build system: Single Cargo package with integration tests and benchmarks.

## Repository Layout

- `src/`: CLI, Git operations, discovery, audit, package, and nested-repo code.
- `tests/`: integration and scenario tests.
- `docs/`: user-facing guides and examples.
- `infra/aw/`: pinned Agent Workspace submodule.
- `config/aw/`: repo-owned Agent Workspace profile.

## Commands

```bash
cargo build --release           # build optimized CLI
cargo test                      # run full test suite
cargo clippy --all-targets --all-features -- -D warnings
cargo fmt --check               # verify formatting
make aw-install                 # install Agent Workspace and repo adapters
make aw-doctor                  # validate Agent Workspace repo setup
```

## Git And Package Workflow

- Git command policy: do not discard user changes; preserve dirty worktrees.
- Commit command: use normal Git unless the user requests an AW commit workflow.
- Package-manager mutation command: `cargo add`, `cargo rm`, or direct `Cargo.toml` edits as appropriate.
- Submodule/worktree notes: initialize/update `infra/aw` with `git submodule update --init --recursive`; update via `make aw-update`.
- Commands that require explicit approval: destructive Git commands, credential changes, publishing, or history rewrites.

## Testing

- Test framework: Rust unit tests, integration tests, and doc tests through Cargo.
- Browser/rendering test command: none.
- Full regression command: `cargo test`.
- Targeted test guidance: use `cargo test --test <name>` for integration targets.
- Known report viewers: none.

## Dev Server

- Start: not applicable.
- Stop: not applicable.
- Logs: not applicable.
- Local URL: not applicable.
- Ports: none.

## Documentation

- Human-facing proposals: keep README and `docs/guides/commands.md` aligned with CLI behavior.
- LLM-facing docs: `.agents.local/project.md` for local facts; `.agents/` for shared agent behavior.
- Scratch/debug artifacts: keep temporary files out of the repo.
- Changelog: update `CHANGELOG.md` for release-facing behavior changes.
- Index files to update: `README.md`, `docs/README.md`, and relevant guide pages.

## Code Standards Overrides

- Import rules: prefer existing module boundaries and helpers.
- File naming: follow existing Rust module names.
- Type/JSDoc expectations: document public command behavior when it clarifies CLI contracts.
- UI/framework conventions: terminal output should stay concise and actionable.
- Security/privacy notes: avoid logging secrets, tokens, or credential material.

## Local Cautions

- Active generated folders: `target/` and `infra/aw/target/`.
- Slow/expensive commands: full `cargo test` and first AW build can compile many dependencies.
- Shared resources: tests mutate current working directory and use a global test lock.
- Deployment or credential constraints: package publishing commands may require registry credentials.
