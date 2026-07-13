# Performance Tuning

Use these controls when a host or Git provider cannot sustain the default
parallelism.

## Concurrency Control

By default, `repos` uses the host's available parallelism plus two concurrent
Git operations.

- Use `--jobs N` to set an explicit concurrency limit.
- Higher isn't always better: excessive concurrency can lead to local disk I/O bottlenecks or remote rate limiting.

## SSD vs HDD

Discovery and Git status are I/O intensive. An SSD generally reduces directory
walking and repository metadata latency.

## Rate Limiting

If you encounter `403 Forbidden` or rate-limit errors from a Git provider:
1. Reduce concurrency using `--jobs 4`.
2. Verify the credential used by the configured remote.
3. For GitHub visibility checks, verify `gh auth status`.

## LFS Optimization

If your repositories use Git LFS, `repos` handles them automatically. To speed up LFS operations:
- Ensure `git-lfs` is installed and up-to-date.
- `repos sync` and `repos pull` pre-fetch LFS objects before checkout to avoid sequential blocking.
