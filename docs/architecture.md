# Architecture

`repos` is a Rust CLI and library for operating on many Git repositories under
one directory. The binary owns argument parsing; reusable behavior lives in the
library crate.

## Source Layout

```text
src/
├── main.rs                 CLI arguments and command dispatch
├── lib.rs                  Library boundary
├── commands/               User-facing workflows
│   ├── sync.rs             Push, pull, and two-way sync orchestration
│   ├── save.rs             Stage, commit, and push workflow
│   ├── staging.rs          Stage, unstage, commit, and status commands
│   ├── config.rs           Git identity synchronization
│   ├── doctor.rs           Read-only repository diagnostics
│   ├── audit.rs            Audit command orchestration
│   └── publish/            Publish planning and execution
├── core/                   Discovery, progress, concurrency, and statistics
├── git/                    Git command execution and result classification
├── audit/                  Secret and repository-hygiene scanners and fixes
├── package/                Cargo, npm, and PyPI package adapters
├── subrepo/                Nested repository validation, drift, and sync
└── utils/                  Filesystem and terminal helpers
```

`src/main.rs` imports the library crate. It does not redeclare the library
modules, so each module and unit test is compiled once.

## Command Flow

Most fleet commands follow the same sequence:

1. Discover repositories below the current directory.
2. Build a processing context with shared progress and statistics.
3. Run repository work concurrently under semaphores.
4. Classify every result and update aggregate statistics.
5. Print a final report and return an error when hard failures occurred.

`repos sync` runs the pull workflow first and the push workflow second. Both
results are retained, so a push failure cannot hide a pull failure or vice
versa.

## Repository Discovery

`core::discovery` uses `ignore::WalkBuilder` with a parallel walker. It follows
directory symlinks, skips dependency/build directories and `.git` internals,
and limits traversal depth.

Discovered paths are deduplicated and sorted before names are assigned. When
multiple paths have the same directory name, the lexically first path owns the
base name and later paths receive `-2`, `-3`, and so on. This makes command
targets stable across runs even though walking is concurrent.

## Git Execution

`git::operations::run_git` is the common async Git process boundary. It:

- uses the repository as the process working directory;
- disables terminal and Git Credential Manager prompts;
- supplies batch-mode SSH only when the caller has not set
  `GIT_SSH_COMMAND`;
- preserves each configured remote URL and transport by default;
- kills child processes when their future is dropped;
- enforces a 180-second timeout; and
- returns command success, stdout, and stderr separately.

With `repos.transportPolicy=ssh-only`, fetch and push inspect Git's effective
URL, including `pushurl` and `insteadOf` rewrites, before any network command.
HTTP(S) is rejected with safe remote context, and network commands clear Git
credential helpers so helpers such as macOS `osxkeychain` cannot open UI.

Network commands retry transient failures with bounded backoff. Normal Git
nonzero statuses are classified by callers and become repository failures when
the requested operation could not be completed.

## Concurrency

The default Git concurrency is the host's available parallelism plus two.
`--jobs` sets an explicit limit and `--sequential` sets it to one.

Push and pull use a pipelined model per repository:

```text
fetch permit -> inspect state -> release fetch permit
write permit -> push or pull -> record result -> release write permit
```

Fetches may use up to twice the configured Git concurrency, capped at 24.
Secret scanning is limited to one repository and hygiene scanning to three.

## Safety Boundaries

- Push and pull inspect remotes, branches, upstreams, and worktree state before
  mutation.
- Pull uses fast-forward-only behavior unless the caller requests rebase.
- Missing or inaccessible remotes are failures, not clean/synced results.
- `repos doctor` probes every configured remote with `git ls-remote` and exits
  nonzero when it finds blockers.
- Audit scanners distinguish a clean scan from an inspection failure.
- History-rewriting audit fixes require a clean repository and, when a remote
  exists, a reachable configured upstream that is not ahead of local `HEAD`.
- Downloaded installer scripts are executed only after checksum verification.

## Nested Repositories

Nested repositories are ordinary Git repositories inside parent repositories,
not Git submodules. Validation groups them by a normalized remote identity.
Equivalent GitHub HTTPS and SSH URLs share a group; case is preserved for paths
on hosts where repository paths may be case-sensitive.

Sync and update select a single remote group by nested repository name. If the
same name refers to different remotes, the command stops as ambiguous before
checking out any commit. Normal updates also require the remote target to be a
fast-forward from each current commit, so divergent local commits stay checked
out for manual review.

## Package Publishing

`commands::publish::planner` discovers package managers, applies visibility and
exact repository-name filters, and blocks unsafe dirty or uninspectable
repositories. `executor` delegates publication to the package adapter and only
creates/pushes a tag after a successful publish.

Package adapters implement the `PackageManager` trait for Cargo, npm, and PyPI.
Registry credentials remain owned by those package-manager tools.

## Public Boundary

`lib.rs` exposes command plumbing for integration and automation. Stable core
and Git entry points are curated through `core::api` and `git::api`; callers
should prefer those re-exports over internal module paths.

## Verification

The repository uses unit tests for classification and formatting, integration
tests with temporary Git repositories and local bare remotes, stress tests for
discovery, and Criterion benchmarks for discovery/context hot paths.

The standard verification gates are:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
```
