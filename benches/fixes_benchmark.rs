use criterion::{criterion_group, criterion_main, Criterion};
use goobits_repos::audit::fixes::{apply_fixes, FixOptions};
use goobits_repos::audit::hygiene::{HygieneStatistics, HygieneViolation, ViolationType};
use goobits_repos::audit::hygiene::report::HygieneStatus;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn setup_repo() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    // Init git repo
    Command::new("git")
        .arg("init")
        .arg("-q") // Quiet
        .current_dir(root)
        .output()
        .expect("git init failed");

    // Configure user for commit
    Command::new("git")
        .args(["config", "user.email", "you@example.com"])
        .current_dir(root)
        .output()
        .expect("git config email failed");
    Command::new("git")
        .args(["config", "user.name", "Your Name"])
        .current_dir(root)
        .output()
        .expect("git config name failed");

    // Create a file that should be ignored
    let ignored_file = root.join("node_modules").join("foo.js");
    fs::create_dir_all(ignored_file.parent().unwrap()).unwrap();
    fs::write(&ignored_file, "console.log('hello');").unwrap();

    // Track it (so it is a violation)
    Command::new("git")
        .args(["add", "."])
        .current_dir(root)
        .output()
        .expect("git add failed");

    Command::new("git")
        .args(["commit", "-m", "initial", "--quiet"])
        .current_dir(root)
        .output()
        .expect("git commit failed");

    temp_dir
}

async fn run_apply_fixes(path: &std::path::Path) {
    let mut stats = HygieneStatistics::new();
    let violation = HygieneViolation {
        file_path: "node_modules/foo.js".to_string(),
        violation_type: ViolationType::GitignoreViolation,
        size_bytes: None,
    };

    // Use update to populate stats
    stats.update(
        "test-repo",
        path.to_str().unwrap(),
        &HygieneStatus::Violations,
        "found violations",
        vec![violation]
    );

    let options = FixOptions {
        interactive: false,
        fix_gitignore: true,
        fix_large: false,
        fix_secrets: false,
        untrack_files: true,
        dry_run: false, // We want to execute IO
        skip_confirm: true,
        target_repos: None,
    };

    let _ = apply_fixes(&stats, options).await;
}

fn bench_fixes(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("fixes");
    group.sample_size(10); // Reduce sample size to save time

    group.bench_function("apply_fixes_gitignore", |b| {
        b.to_async(&runtime).iter_custom(|iters| async move {
            let mut total_duration = std::time::Duration::new(0, 0);
            for _ in 0..iters {
                let temp_dir = setup_repo();
                let path = temp_dir.path().to_path_buf();

                let start = std::time::Instant::now();
                run_apply_fixes(&path).await;
                let elapsed = start.elapsed();
                total_duration += elapsed;
            }
            total_duration
        })
    });
    group.finish();
}

criterion_group!(benches, bench_fixes);
criterion_main!(benches);
