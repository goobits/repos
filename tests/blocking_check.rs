use goobits_repos::core::init_command;
use std::fs;
use std::time::{Duration, Instant};
use tempfile::TempDir;

#[tokio::test] // Default is single threaded scheduler
async fn test_blocking_discovery() {
    // Setup - Create many repos to make discovery slow
    let temp_dir = TempDir::new().unwrap();
    let root = temp_dir.path();

    println!("Setting up repositories...");
    for i in 0..1000 {
        let repo_path = root.join(format!("repo-{}", i));
        fs::create_dir(&repo_path).unwrap();
        fs::create_dir(repo_path.join(".git")).unwrap();
    }

    // Change current directory to temp dir so init_command scans it
    let _orig_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(root).unwrap();

    let start = Instant::now();

    // Run concurrent heartbeat task
    let heartbeat_handle = tokio::spawn(async move {
        let mut max_delay = Duration::from_secs(0);
        let start_time = Instant::now();
        let mut last_tick = start_time;

        loop {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let now = Instant::now();

            // Check if we were cancelled/stopped
            if now.duration_since(start_time) > Duration::from_secs(5) {
                break;
            }

            let delay = now.duration_since(last_tick);
            // Expected delay is ~10ms + overhead.
            // If we calculate delay between *expected* wake up and actual wake up:
            // But checking inter-tick time is easier.
            // Ideally it should be close to 10ms + execution time.
            // If it is > 100ms, something blocked us.
            if delay > max_delay {
                max_delay = delay;
            }
            last_tick = now;
        }
        max_delay
    });

    // Run the command under test
    println!("Running init_command...");
    // This is synchronous and will block the single thread
    let _ = init_command("Scanning...").await;

    let duration = start.elapsed();
    println!("init_command took: {:?}", duration);

    // Cancel heartbeat (it will exit on loop check or we can abort)
    heartbeat_handle.abort();
    // We can't get result if aborted?
    // Wait, if it was blocked, it never ran.
    // If we abort, we can't get the return value easily.

    // Better: let it run for a bit more then check shared state?
    // Or just await it with a timeout, but we need it to stop.
}

#[test]
fn test_blocking_discovery_measure() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    rt.block_on(async {
        println!("Main thread: {:?}", std::thread::current().id());
        // Setup - Create many repos to make discovery slow
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();

        for i in 0..1000 {
            let repo_path = root.join(format!("repo-{}", i));
            fs::create_dir(&repo_path).unwrap();
            fs::create_dir(repo_path.join(".git")).unwrap();
        }

        let _orig_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(root).unwrap();

        let start = Instant::now();

        // Use an Atomic to track max delay
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;

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

                println!("Tick delay: {}ms", delay_ms);

                let current_max = max_delay_clone.load(Ordering::Relaxed);
                if delay_ms > current_max {
                    max_delay_clone.store(delay_ms, Ordering::Relaxed);
                }
                last_tick = now;
            }
        });

        // Yield to let heartbeat start
        tokio::time::sleep(Duration::from_millis(1)).await;

        println!("Running init_command...");
        // This will block the single thread
        // std::thread::sleep(Duration::from_millis(100));
        let _ = init_command("Scanning...").await;
        let duration = start.elapsed();

        // Yield to let the heartbeat task process (it should process now)
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Check max delay
        let delay = max_delay_ms.load(Ordering::Relaxed);
        println!("Init command duration: {:?}", duration);
        println!("Max heartbeat delay: {} ms", delay);

        heartbeat_handle.abort();
        std::env::set_current_dir(_orig_dir).unwrap();

        if delay > 90 {
            println!("Confirmed: BLOCKING behavior detected.");
            panic!("Expected non-blocking behavior but saw blocking");
        } else {
            println!("Observed: NON-BLOCKING behavior.");
        }
    });
}
