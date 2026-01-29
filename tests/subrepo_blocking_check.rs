use goobits_repos::subrepo::validation::validate_subrepos;
use std::fs;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[test]
fn test_subrepo_validation_blocking() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        println!("Main thread: {:?}", std::thread::current().id());
        // Setup - Create many repos to make discovery slow
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        println!("Setting up 1000 repositories...");
        for i in 0..1000 {
            let repo_path = root.join(format!("repo-{}", i));
            fs::create_dir(&repo_path).unwrap();
            // Initialize as git repo so find_repos detects it
            // We can just make a .git directory
            fs::create_dir(repo_path.join(".git")).unwrap();
        }

        let _orig_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(root).unwrap();

        let start = Instant::now();

        // Use an Atomic to track max delay in the heartbeat
        let max_delay_ms = Arc::new(AtomicU64::new(0));
        let max_delay_clone = max_delay_ms.clone();

        let heartbeat_handle = tokio::spawn(async move {
            println!("Heartbeat thread: {:?}", std::thread::current().id());
            let mut last_tick = Instant::now();
            loop {
                tokio::time::sleep(Duration::from_millis(10)).await;
                let now = Instant::now();
                let delay = now.duration_since(last_tick);
                let delay_ms = delay.as_millis() as u64;

                // Expected is ~10ms. If it's > 20ms, we have delay.
                // We track max delay to catch the blocking period.
                let current_max = max_delay_clone.load(Ordering::Relaxed);
                if delay_ms > current_max {
                    max_delay_clone.store(delay_ms, Ordering::Relaxed);
                }
                last_tick = now;
            }
        });

        // Yield to let heartbeat start
        tokio::time::sleep(Duration::from_millis(5)).await;

        println!("Running validate_subrepos (async)...");

        // This should NOT BLOCK the single thread because it uses spawn_blocking
        let _ = validate_subrepos().await;

        let duration = start.elapsed();
        println!("validate_subrepos took: {:?}", duration);

        // Yield to let the heartbeat task process
        tokio::time::sleep(Duration::from_millis(50)).await;

        heartbeat_handle.abort();
        std::env::set_current_dir(_orig_dir).unwrap();

        let delay = max_delay_ms.load(Ordering::Relaxed);
        println!("Max heartbeat delay: {} ms", delay);

        if delay > 100 {
             println!("Confirmed: BLOCKING behavior detected (Failed).");
             panic!("Expected non-blocking behavior but saw blocking");
        } else {
             println!("Observed: NON-BLOCKING behavior (Success).");
        }
    });
}
