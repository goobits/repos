# Command Reference

`repos` manages a fleet of Git repositories from one directory tree.

The CLI is intent-first for daily work and keeps granular Git controls available
when you need them.

Daily model:

```bash
repos status
repos save "message"
repos sync
```

Safety model:

- `save` stages tracked changes only unless you opt into untracked files.
- `sync` pulls safe remote changes, pushes local commits, and skips dirty repositories instead of stashing implicitly.
- branch publishing uses `--auto-upstream`, not overloaded force wording.
- mutating daily workflows expose `--dry-run` where practical.

Repository-oriented report sections are sorted by path, keeping a top-level
project and its nested packages together. Nested package drift is sorted by
package name, then by each copy's project path.

Transfer and sync reports combine actionable details under `Needs Attention by
Project`. Within each project, `!` marks failures, `·` marks skipped
repositories, and `~` marks non-exclusive follow-up work. Nested drift remains
its own package-oriented section. A compact `Final Totals` footer repeats the
outcome counts after all details, so long reports end with the result of the run;
`sync` also repeats its pulled and pushed repository and commit totals there.

## Overview

```text
repos - Fleet-scale Git orchestration for humans

USAGE:
  repos <command> [options]

EVERYDAY:
  status      Understand repository state
  save        Stage tracked changes, commit, and push
  sync        Pull safe remote changes, push local commits, and report nested drift

CONTROL:
  fetch       Refresh remote refs without changing local branches
  stage       Stage matching files
  unstage     Unstage matching files
  commit      Commit currently staged changes
  push        Push unpushed commits
  pull        Pull remote changes

MAINTENANCE:
  audit       Scan for secrets and hygiene issues
  publish     Publish detected packages
  doctor      Diagnose remotes, upstreams, worktrees, and nested drift

ADVANCED:
  nested      Manage nested repository drift
  config      Sync Git identity/config
```

## Everyday

### `repos status`

Show repository state across the fleet.

```bash
repos status
repos status --needs-work
repos status --skipped
repos status tunajack.com
repos status ./packages/logger
```

Pass one or more repository names or paths to inspect only those repositories.

Status refreshes the current upstream before comparing commits, without moving
the local branch or worktree. It reports branch, worktree, and current remote
state per repository; ahead, behind, and diverged branches all need work and
include the exact next command. Under the SSH-only transport policy, HTTP(S)
remotes are reported with an SSH conversion command before credential helpers
can run. When exactly one repository is checked, dirty files are listed below
the summary.

Options:

| Option | Description |
|---|---|
| `--needs-work` | Show dirty, ahead, behind, diverged, missing-remote, missing-upstream, or failed repos |
| `--dirty` | Show repos with uncommitted worktree changes |
| `--no-remote` | Show repos without a configured remote |
| `--no-upstream` | Show repos without an upstream branch |
| `--failed` | Show repos where status inspection failed |
| `--skipped` | Show repos that would be skipped by `repos push` because there is nothing pushable |

### `repos save`

Stage tracked changes, commit, and push in one command. This is the humane
replacement for the common `stage -> commit -> push` loop.

```bash
repos save "Update docs"
```

Safe default:

- Stages tracked modifications and deletions only.
- Does not stage untracked files by default.
- Commits repositories with staged changes.
- Pushes successful commits.
- Skips branches without upstream unless `--auto-upstream` is passed.
- Saves nested children before parents so parent gitlinks record the new child commits.
- Before pushing a parent gitlink, verifies that the exact child commit is reachable from fetched remote refs.

Options:

| Option | Description |
|---|---|
| `-u`, `--include-untracked` | Include untracked files |
| `-a`, `--all` | Stage all non-ignored changes |
| `--auto-upstream` | Set upstream for branches without tracking |
| `--dry-run` | Print planned save actions without mutating repositories |

Examples:

```bash
repos save "Update docs"
repos save "Add assets" --include-untracked
repos save "Initial project state" --all
repos save "Publish branch" --auto-upstream
repos save "Preview save" --dry-run
```

### `repos sync`

Pull safe remote changes using rebase, then push local commits. This is the
humane daily command for “reconcile my workspace with remotes.”

```bash
repos sync
```

Default behavior:

- Fetches remotes.
- Pulls with rebase.
- Pushes local commits after the pull phase.
- Scans once and prints one final report with exclusive repo outcomes plus named pull/push transfers.
- Skips dirty repositories instead of stashing implicitly.
- Reports nested repository drift.
- Leaves directional behavior available through `repos pull` and `repos push`.
- Pulls parents before children, then pushes children before parents.

Options:

| Option | Description |
|---|---|
| `-v`, `--verbose` | Show detailed progress |
| `-c`, `--show-changes` | Show file changes in dirty repositories |
| `--auto-upstream` | Set upstream during the push phase for branches without tracking |
| `--no-drift-check` | Skip nested drift check |

Advanced options are hidden from main help but still available:

| Option | Description |
|---|---|
| `-j`, `--jobs <N>` | Limit concurrency |
| `--sequential` | Run one repository at a time |

## Control

### `repos fetch`

Fetch every configured remote in every discovered repository without merging,
rebasing, checking out, or changing a worktree.

```bash
repos fetch
repos fetch --verbose
```

The final report shares the push/pull contract: exclusive `Fetched`, `Up to
date`, `Failed`, and `Skipped` outcomes add up to `Checked`; updated and skipped
repositories are named, and failures include path, remote context, and a next
action. The fetched count is the number of remote-tracking refs or tags changed
during this run.

Advanced options are hidden from main help but still available:

| Option | Description |
|---|---|
| `-j`, `--jobs <N>` | Limit concurrency |
| `--sequential` | Run one repository at a time |

### `repos stage`

Stage files matching a pattern across repositories.

```bash
repos stage "*.md"
repos stage "README.md"
repos stage "*"
```

### `repos unstage`

Unstage files matching a pattern across repositories.

```bash
repos unstage "*.md"
repos unstage "*"
```

### `repos commit`

Commit currently staged changes.

```bash
repos commit "Fix typos"
repos commit "Bump version" --include-empty
```

Options:

| Option | Description |
|---|---|
| `--include-empty` | Create empty commits in repositories without staged changes |

### `repos push`

Push unpushed commits.

```bash
repos push
repos push --auto-upstream
```

Nested children are pushed before parents. Repositories at the same dependency
level remain concurrent, even with `--jobs`. For Git submodules, a parent push
is blocked unless the exact indexed child commit is reachable from freshly
fetched child remote refs; a normal published detached checkout is allowed.

The final report uses exclusive `Pushed`, `Up to date`, `Failed`, and `Skipped`
outcomes that add up to `Checked`. Pushed repositories are named; failures,
skips, and local follow-up work are grouped by top-level project with a path,
reason, and next action. Nested drift remains a separate package-first view.

Options:

| Option | Description |
|---|---|
| `--auto-upstream` | Set upstream for branches without tracking |
| `-v`, `--verbose` | Show detailed progress |
| `-c`, `--show-changes` | Show file changes in dirty repositories |
| `--no-drift-check` | Skip nested drift check |

Advanced options are hidden from main help but still available:

| Option | Description |
|---|---|
| `-j`, `--jobs <N>` | Limit concurrency |
| `--sequential` | Run one repository at a time |

### `repos pull`

Granular pull command.

```bash
repos pull
repos pull --rebase
```

Parents are pulled before nested children so updated parent state is established
first. This ordering does not replace `git submodule update` when a parent
changes a submodule pointer.

The final report mirrors `repos push`: exclusive outcomes, named pulled
repositories, and one project-grouped section for actionable failures, skips,
and non-exclusive local follow-up work.

Options:

| Option | Description |
|---|---|
| `--rebase` | Use `git pull --rebase` |
| `-v`, `--verbose` | Show detailed progress |
| `-c`, `--show-changes` | Show file changes in dirty repositories |
| `--no-drift-check` | Skip nested drift check |

Advanced options are hidden from main help but still available:

| Option | Description |
|---|---|
| `-j`, `--jobs <N>` | Limit concurrency |
| `--sequential` | Run one repository at a time |

## Maintenance

### `repos audit`

Scan for secrets and repository hygiene issues.

```bash
repos audit
repos audit --verify
repos audit --json
repos audit --fix-gitignore
repos audit --fix-all --dry-run
```

Options:

| Option | Description |
|---|---|
| `--install-tools` | Install required tools without prompting |
| `--verify` | Verify discovered secrets are active |
| `--json` | Output JSON |
| `--interactive` | Choose fixes interactively |
| `--fix-gitignore` | Add missing `.gitignore` entries |
| `--fix-large` | Remove large files from history |
| `--fix-secrets` | Remove secrets from history |
| `--fix-all` | Apply all available fixes |
| `--dry-run` | Preview fixes |
| `--repos <repo1,repo2>` | Target specific repositories |

Fixes run children before parents and repeat safety checks immediately before
mutation. Bulk history rewrites are refused when the selected repositories
cross a parent/submodule dependency; rewrite and validate that dependency chain
explicitly instead.

### `repos publish`

Publish detected packages to registries.

```bash
repos publish
repos publish --dry-run
repos publish --tag
repos publish my-app my-lib
```

Real publishes require each release commit to be clean, fully pushed, and not
behind its upstream. Local package dependencies declared in Cargo, npm, or
Python manifests publish first; independent packages in a dependency wave stay
concurrent. Duplicate registry identities and dependency cycles fail before any
registry mutation. With `--tag`, an existing tag must already point to the
release commit.

Options:

| Option | Description |
|---|---|
| `--dry-run` | Preview without publishing |
| `--tag` | Create and push Git tags after publish |
| `--allow-dirty` | Allow publishing dirty repositories |
| `--all` | Publish public and private repositories |
| `--public-only` | Publish public repositories only |
| `--private-only` | Publish private repositories only |

### `repos doctor`

Diagnose common fleet blockers without mutating anything.

```bash
repos doctor
```

Checks:

- Detached HEADs.
- Missing remotes.
- Effective fetch and push transport for every configured remote, including a separate `pushurl`.
- Access to non-HTTP fetch remotes (`git ls-remote --heads`).
- Missing upstream tracking.
- Dirty worktrees.
- Conflicts.
- Nested repository drift.

`doctor` is read-only and exits nonzero when it finds a blocker. Its sorted
final report separates healthy repositories, warnings, blockers, and nested
drift; every warning/blocker includes a path and next action. HTTP(S) access
checks are skipped under the default policy so credential helpers such as
macOS Keychain are not invoked merely to diagnose the repository.

To guarantee that fleet commands do not consult HTTP credential helpers such as
macOS Keychain, enable SSH-only policy once:

```bash
git config --global repos.transportPolicy ssh-only
repos doctor
```

The policy blocks effective HTTP(S) fetch and push URLs before access checks
and reports the repository, sanitized remote identity, and exact
`git remote set-url` command. Use `REPOS_TRANSPORT_POLICY=preserve` for a
one-command exception.

## Advanced

### `repos nested`

Manage nested repository drift.

```bash
repos nested validate
repos nested status
repos nested status --all
repos nested sync my-lib --to abc1234
repos nested sync my-lib --to abc1234 --stash
repos nested update my-lib
```

Subcommands:

| Subcommand | Description |
|---|---|
| `validate` | Validate nested repository setup |
| `status` | Show nested drift |
| `sync` | Preflight every eligible copy, then sync them to one requested commit |
| `update` | Resolve one remote target, then fast-forward eligible copies; skip dirty or divergent copies |

### `repos config`

Sync Git identity across repositories.

```bash
repos config --from-global
repos config --from-current
repos config --name "Alice" --email "alice@example.com"
repos config --from-global --dry-run
repos config --from-global --yes
```

Options:

| Option | Description |
|---|---|
| `--name <name>` | Set Git user name |
| `--email <email>` | Set Git user email |
| `--from-global` | Use global Git config as source |
| `--from-current` | Use current repository config as source |
| `--yes` | Apply without prompting |
| `--dry-run` | Preview changes |
