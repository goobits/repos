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
│   ├── sync.rs             Shared transfer setup and two-way sync orchestration
│   ├── sync/               Focused fetch, push, pull, and progress pipelines
│   ├── save.rs             Stage, commit, and push workflow
│   ├── staging.rs          Stage, unstage, commit, and status commands
│   ├── staging/            Single-repo mutations plus status inspection/rendering
│   ├── config.rs           Git identity synchronization
│   ├── doctor.rs           Read-only repository diagnostics
│   ├── doctor/             Batched config inspection and doctor reporting
│   ├── audit.rs            Audit command orchestration
│   └── publish/            Publish planning and execution
├── core/                   Discovery, progress, concurrency, and statistics
│   ├── report/sync.rs      Combined pull/push reporting
│   └── stats/              Transfer state, reporting, and safe text formatting
├── git/                    Git command execution and result classification
│   └── operations/         Fetch/push/pull, remote, LFS, visibility, and worktree operations
├── audit/                  Secret and repository-hygiene scanners and fixes
│   ├── scanner/            Secret reports and checksum-verified tool installation
│   └── fixes/              Gitignore updates, history safety, and secret removal
├── package/                Cargo, npm, and PyPI package adapters
├── subrepo/                Nested repository validation, drift, and sync
│   ├── status/             Concise formatting and detailed status rendering
│   └── sync/               Git primitives and nested mutation reports
└── utils/                  Filesystem and terminal helpers
```

`src/main.rs` imports the library crate. It does not redeclare the library
modules, so each module and unit test is compiled once.

Production responsibilities are kept below 500 lines per source module. Larger
physical files only exceed that boundary when they contain an inline
`#[cfg(test)]` section, which is excluded from the production module size.

## Command Flow

Most fleet commands follow the same sequence:

1. Discover repositories below the current directory.
2. Build a processing context with shared progress and statistics.
3. Build parent/child dependency waves when the operation can affect nested state.
4. Run independent repository work concurrently under semaphores.
5. Classify every result and update aggregate statistics.
6. Print a final report and return an error when hard failures occurred.

`repos sync` runs the pull workflow first and the push workflow second. Both
results are retained, so a push failure cannot hide a pull failure or vice
versa.

## Repository Discovery

`core::discovery` uses `ignore::WalkBuilder` with a parallel walker. It follows
directory symlinks, skips curated dependency/build directories and `.git`
internals, and scans to arbitrary depth. Git, global, and tool-specific ignore
files are not fleet inventory controls: an ignored nested repository is still
discovered and ordered. Command scope comes from the requested scan root, and
`.reposignore` provides explicit per-tree fleet exclusions using gitignore
pattern syntax.

Discovered paths are canonicalized and deduplicated by physical checkout before
names are assigned, so a symlink alias cannot run a fleet operation twice. When
different repositories have the same directory name, the lexically first path
owns the base name and later paths receive `-2`, `-3`, and so on. This makes
command targets stable across runs even though walking is concurrent.

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

The default Git concurrency is the host's available parallelism plus two, capped
at 32. `--jobs` is an explicit override and `--sequential` sets it to one.

Push and pull use a pipelined model per repository:

```text
fetch permit -> inspect state -> release fetch permit
write permit -> push or pull -> record result -> release write permit
```

Operation results carry structured transfer quantities alongside human-readable
messages. Summaries consume those values directly. Atomic counters and focused
collection locks make the statistics collector safe to share without a global
outer mutex.

Fetches may use up to twice the configured Git concurrency, capped at 24.
Pipelines are grouped into topology waves. Commit, save, and push execute the
deepest nested repositories first; pull executes parents first. Independent
repositories in a wave still run concurrently. Only indexed Git gitlinks are
hard publication dependencies—ordinary nested repositories and linked
worktrees receive deterministic ordering without being treated as submodules.

One immutable topology snapshot supplies canonical nearest-parent relationships
and batched parent-index gitlinks to both transfer phases and nested reporting.
Gitlink inspection failures are retained as errors rather than interpreted as
the absence of a submodule. Parents with `.gitmodules` are also inspected so
declared but uninitialized submodules remain visible outside checkout discovery.

The standalone `repos fetch` command uses the common Git concurrency limit,
updates every configured remote, and never changes a local branch or worktree.
Secret scanning is limited to one repository and hygiene scanning to three.
Publish discovery and manifest inspection use a separate eight-repository cap so
large fleets do not create unbounded GitHub CLI or filesystem work.

## Safety Boundaries

- Push and pull inspect remotes, branches, upstreams, and worktree state before
  mutation.
- Pull uses fast-forward-only behavior unless the caller requests rebase.
- Missing or inaccessible remotes are failures, not clean/synced results.
- `repos doctor` probes every configured remote with `git ls-remote` and exits
  nonzero when it finds blockers. It batches raw configured URL inspection once
  per repository, then checks effective fetch and push URLs separately so Git
  rewrite rules and transport policy remain visible.
- Audit scanners distinguish a clean scan from an inspection failure.
- History-rewriting audit fixes require a clean repository and, when a remote
  exists, a reachable configured upstream that is not ahead of local `HEAD`.
- Parent pushes containing Git gitlinks require the exact indexed child commit
  to be reachable from freshly fetched remote-tracking refs. A normal detached
  submodule checkout is therefore safe when its commit is published, while a
  local-only child commit blocks the parent.
- Bulk history rewrites are refused when the selected set crosses a
  parent/submodule dependency, because rewriting the child invalidates the
  parent's historical gitlinks.
- Downloaded installer scripts are executed only after checksum verification.

## Nested Repositories

Nested drift inventory includes every Git repository discovered below another
fleet repository. Each checkout is classified as an independent repository, a
registered Git submodule, or a linked worktree. Submodule gitlinks still drive
dependency ordering and publication safety. Nested validation derives nearest
parent relationships from the same fleet inventory so a physical checkout is
counted once and `.reposignore` has identical scope. Validation groups nested
repositories by a normalized remote identity.
Equivalent GitHub HTTPS and SSH URLs share a group; case is preserved for paths
on hosts where repository paths may be case-sensitive. Exact HEAD, origin, and
worktree inspection runs through a bounded eight-worker pool; results are
restored to fleet order before errors or reports are emitted.

Sync and update select a single remote group by nested repository name. If the
same name refers to different remotes, the command stops as ambiguous before
checking out any commit. Each batch resolves one immutable target and preflights
its availability in every eligible copy before moving a checkout. Normal
updates also require the remote target to be a fast-forward from each current
commit, so divergent local commits stay checked out for manual review.

## Package Publishing

`commands::publish::planner` discovers package managers, applies visibility and
exact repository-name filters, and blocks unsafe dirty or uninspectable
repositories. A real publish also requires the release commit to match its
upstream after fetch. Each built-in adapter parses its static manifest once;
malformed manifests and dynamic `setup.py` metadata fail closed. `executor`
derives local registry dependencies from
manifests, rejects duplicate identities/cycles before mutation, and publishes
dependency waves in order. Tags are created or pushed only when they resolve to
the planned release commit.

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
