# Performance Tuning

`repos` is optimized for speed, but there are several ways to tune it for your specific environment.

## Concurrency Control

By default, `repos` uses `num_cpus + 2` concurrent operations for git commands. For very large deployments (1000+ repos), you might want to adjust this.

- Use `--jobs N` to set an explicit concurrency limit.
- Higher isn't always better: excessive concurrency can lead to local disk I/O bottlenecks or remote rate limiting.

## SSD vs HDD

Discovery and git status are extremely I/O intensive. `repos` uses parallel walking which significantly benefits from SSD random access speeds.

## Rate Limiting

If you encounter `403 Forbidden` or `Secondary Rate Limit` errors from GitHub:
1. Reduce concurrency using `--jobs 4`.
2. Ensure you are authenticated with the `gh` CLI (`gh auth status`). `repos` uses your authenticated session via `gh` for visibility checks.

## LFS Optimization

If your repositories use Git LFS, `repos` handles them automatically. To speed up LFS operations:
- Ensure `git-lfs` is installed and up-to-date.
- `repos pull` pre-fetches LFS objects before checkout to avoid sequential blocking.
