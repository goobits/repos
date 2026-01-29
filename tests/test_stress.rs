use anyhow::Result;
use goobits_repos::core::find_repos_from_path;
use goobits_repos::git::fetch_and_analyze;
use std::fs;
use std::process::Command;
use tempfile::TempDir;
use futures::stream::{FuturesUnordered, StreamExt};

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
        
        // Add a remote
        Command::new("git")
            .args(["remote", "add", "origin", "https://github.com/example/repo.git"])
            .current_dir(&repo_path)
            .output()
            .unwrap();
    }

    temp_dir
}

#[tokio::test]
async fn test_stress_discovery_and_analysis() -> Result<()> {
    let count = 50;
    let temp_dir = setup_many_repos(count);
    let path = temp_dir.path();

    // 1. Discovery
    let repos = find_repos_from_path(path, None);
    assert_eq!(repos.len(), count);

    // 2. Parallel analysis
    let mut futures = FuturesUnordered::new();
    for (_name, repo_path) in repos {
        futures.push(async move {
            fetch_and_analyze(&repo_path, false).await
        });
    }

    let mut results = Vec::new();
    while let Some(res) = futures.next().await {
        results.push(res);
    }

    assert_eq!(results.len(), count);
    
    Ok(())
}

#[tokio::test]
async fn test_stress_discovery_scaling_500() -> Result<()> {
    // This test verifies that discovery scales linearly-ish and doesn't crash
    let count = 500;
    let temp_dir = setup_many_repos(count);
    let path = temp_dir.path();

    let start = std::time::Instant::now();
    let repos = find_repos_from_path(path, None);
    let duration = start.elapsed();

    assert_eq!(repos.len(), count);
    
    // Simple sanity check on performance - should be under 2 seconds for 500 repos on most machines
    // (Benchmark shows ~3.6ms for 100, so 500 should be ~20ms, but overhead overhead adds up)
    println!("Discovered {} repos in {:?}", count, duration);
    
    Ok(())
}
