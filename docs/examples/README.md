# Examples

Runnable templates and focused workflow examples. For command semantics and
failure behavior, use the [commands reference](../guides/commands.md).

## Templates

- [Security audit workflow](github-actions-security-audit.yml)
- [Package publishing workflow](github-actions-publish.yml)
- [Pre-push audit hook](pre-push-hook.sh)
- [CI/CD integration notes](ci-cd-integration.md)
- [Custom workflows](custom-workflows.md)
- [Performance tuning](performance-tuning.md)

Review workflow permissions, registry credentials, branch names, and tool
installation before using a template in another repository.

## Daily Sync

```bash
repos status --needs-work
repos sync
repos doctor
```

`repos doctor` returns nonzero when a configured remote cannot be reached or
another repository blocker is found.

## Targeted Audit

```bash
repos audit --install-tools --verify --repos api,web
repos audit --fix-gitignore --repos api --dry-run
```

An incomplete secret or hygiene scan returns nonzero. History-rewriting fixes
also stop if their configured upstream cannot be fetched.

## Package Publish

```bash
repos publish api web --dry-run
repos publish api --tag
```

Publish targets are exact discovered repository names. Use `repos status` to
see the names assigned during discovery.

## Nested Repository Drift

```bash
repos nested status
repos nested sync shared-lib --to abc1234 --stash
```

When the same nested repository name refers to different remotes, sync stops as
ambiguous instead of updating both groups.

## Recovery

Audit history fixes print the verified full-ref bundle they create. Recover into
a separate clean directory so the rewritten checkout remains available for
comparison:

```bash
git clone /path/from-output/before.bundle recovered-repository
```

Keep the bundle until the rewrite and its post-checks have been reviewed.
History rewrites and force-pushes are destructive; coordinate any ref updates
with collaborators.
