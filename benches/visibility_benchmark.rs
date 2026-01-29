use criterion::{criterion_group, criterion_main, Criterion};
use goobits_repos::git::get_repo_visibility;
use std::path::Path;
use tokio::runtime::Runtime;

fn bench_visibility_cache_hit(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    // Use a path that will definitely fail git checks and return Unknown
    let path = Path::new("/tmp/goobits_bench_non_existent_path");

    // Warm up the cache
    rt.block_on(async {
        get_repo_visibility(path).await;
    });

    c.bench_function("get_repo_visibility_cache_hit", |b| {
        b.to_async(&rt).iter(|| async {
            get_repo_visibility(path).await
        })
    });
}

criterion_group!(benches, bench_visibility_cache_hit);
criterion_main!(benches);
