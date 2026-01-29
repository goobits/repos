use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;

fn generate_repos(count: usize) -> Vec<(String, PathBuf)> {
    (0..count)
        .map(|i| {
            (
                format!("repo-{}", i),
                PathBuf::from(format!("/path/to/repo-{}", i)),
            )
        })
        .collect()
}

fn bench_context_cloning(c: &mut Criterion) {
    let repo_count = 10000;
    let repos = generate_repos(repo_count);
    let repos_arc = Arc::new(repos.clone());

    let mut group = c.benchmark_group("context_creation");

    group.bench_function("vec_clone", |b| {
        b.iter(|| {
            let _ = black_box(repos.clone());
        })
    });

    group.bench_function("arc_clone", |b| {
        b.iter(|| {
            let _ = black_box(Arc::clone(&repos_arc));
        })
    });

    group.finish();
}

criterion_group!(benches, bench_context_cloning);
criterion_main!(benches);
