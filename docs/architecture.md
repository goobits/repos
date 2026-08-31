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
results are retained. Failures take precedence in the combined outcome, then a
successful transfer, then a skip. Dirty repositories remain skipped through
both phases, so sync never publishes commits from a dirty worktree.

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
HTTP(S) Git URLs are rejected with safe remote context. Explicit Git LFS URL
overrides from Git config or `.lfsconfig` are resolved with `insteadOf` and
`pushInsteadOf`, fingerprinted across pull analysis, and rejected when they use
HTTP(S). The normal data endpoint negotiated through an inspected SSH remote
may still use HTTPS. Strict network commands clear credential helpers and Git
and SSH askpass sources so helpers such as macOS `osxkeychain` cannot open UI.

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
- Upstream-aware transfers fetch the selected remote explicitly. Pull and sync
  pin the analyzed local commit, fetched upstream commit, and pre-fetch
  upstream fork point. They fetch LFS objects for that exact upstream commit
  from the inspected remote, then revalidate the branch, `HEAD`, and worktree
  immediately before direct integration without a second Git fetch. Remote
  names are separated from command options before network operations.
  Automatic upstream creation checks `branch.<name>.pushRemote`,
  `remote.pushDefault`, and `branch.<name>.remote`, then uses `origin` or a sole
  remote; ambiguous multi-remote repositories fail before network mutation.
- A push without an upstream returns before push-transport and LFS side effects
  unless automatic upstream creation was requested. LFS uploads use the local
  source branch; detached checkouts are skipped before publication.
- Pull uses fast-forward-only behavior unless the caller requests rebase. It
  fetches LFS objects for the pinned incoming commit before checkout and retains
  the pre-fetch upstream commit as the rebase fork point.
- Missing or inaccessible remotes are failures, not clean/synced results.
- `repos doctor` inspects every configured remote and probes eligible non-HTTP
  fetch remotes with `git ls-remote`; default-policy HTTP(S) probes are skipped
  so credential helpers cannot run. It exits nonzero when it finds blockers,
  batches raw configured URL inspection once per repository, and checks
  effective fetch and push URLs separately so Git rewrite rules and transport
  policy remain visible.
- Audit target names are exact; a missing requested repository fails instead of
  silently narrowing the scan. Secret findings distinguish verified,
  unverified, and verification-unknown results. In verification mode, verified
  or unknown findings fail, as does any incomplete secret or hygiene scan.
- History-rewriting audit fixes recheck safety before processing each selected
  repository. They require a clean repository and, when a remote exists, a
  reachable configured upstream that is not ahead of local `HEAD`.
- Each history rewrite creates and verifies a private full-ref `before.bundle`,
  combines selected path removal and secret redaction in one rewrite, then
  reruns the selected scans. Rewritten refs are never pushed automatically.
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
exact repository-name filters, and rejects any missing requested name before
package inspection. A real publish rejects dirty repositories unless
`--allow-dirty` was requested. An attached release must match its upstream after
fetch; a detached release is accepted only when the exact local and remote
`v<manifest-version>` tag both resolve to `HEAD`. This provenance check is
independent of `--tag`, which controls post-publish tag creation and push.

Each built-in adapter parses its static manifest once; malformed manifests and
dynamic `setup.py` metadata fail closed. `executor` derives local registry
dependencies from manifests, rejects duplicate identities and dependency cycles
before mutation, and publishes dependency waves in order. PyPI builds use an
invocation-private output directory and pass only the exact artifacts produced
there to Twine, so stale repository `dist/` files are not uploaded.

Package subprocesses have bounded timeouts and cancellation-on-drop. On Unix,
they run in a dedicated process group so cancellation reaches descendants; other
platforms retain direct-child cancellation. A registry command timeout is
reported as outcome unknown because the remote service may have accepted the
publish before local cancellation, so users must check the registry before
retrying.

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

Stable CI runs formatting plus locked verification across every target and
feature:

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets --all-features
cargo test --locked --doc --all-features
cargo audit --deny warnings
```

A separate job runs all targets, features, and doctests on the declared Rust
1.78 minimum. Criterion remains on the Rust-1.78-compatible 0.5 release line.
`make lint` and `make test` use the same breadth for local checks; CI
additionally enforces the lockfile and runs the RustSec audit.
