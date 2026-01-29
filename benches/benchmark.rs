use criterion::{criterion_group, criterion_main, Criterion};
use goobits_repos::core::find_repos_from_path;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn setup_many_repos(count: usize) -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    for i in 0..count {
        let repo_path = root.join(format!("repo-{}", i));
        fs::create_dir(&repo_path).unwrap();
        Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(&repo_path)
            .output()
            .unwrap();
    }

    temp_dir
}

fn bench_discovery(c: &mut Criterion) {
    let count = 100;
    let temp_dir = setup_many_repos(count);
    let path = temp_dir.path().to_path_buf();

    c.bench_function("discovery_100_repos", |b| {
        b.iter(|| find_repos_from_path(&path))
    });

    let count_large = 1000;
    let temp_dir_large = setup_many_repos(count_large);
    let path_large = temp_dir_large.path().to_path_buf();

    c.bench_function("discovery_1000_repos", |b| {
        b.iter(|| find_repos_from_path(&path_large))
    });
}

criterion_group!(benches, bench_discovery);
criterion_main!(benches);
