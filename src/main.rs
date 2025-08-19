use anyhow::Result;
use colored::*;
use futures::stream::{FuturesUnordered, StreamExt};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::process::Command;
use walkdir::WalkDir;


#[derive(Clone)]
enum Status {
    Synced, Pushed, Skip, Error
}

impl Status {
    fn symbol(&self) -> &str {
        match self {
            Status::Synced | Status::Pushed => "🟢",
            Status::Skip => "🟠",
            Status::Error => "🔴",
        }
    }
    
    fn text(&self) -> &str {
        match self {
            Status::Synced => "synced",
            Status::Pushed => "pushed",
            Status::Skip => "skip",
            Status::Error => "error",
        }
    }
}

async fn run_git(path: &Path, args: &[&str]) -> Result<(bool, String, String)> {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .await?;
    
    Ok((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).trim().to_string(),
        String::from_utf8_lossy(&output.stderr).trim().to_string(),
    ))
}

async fn check_repo(path: &Path) -> (Status, String, bool) {
    // Check uncommitted changes
    let has_uncommitted = !run_git(path, &["diff-index", "--quiet", "HEAD", "--"])
        .await
        .map(|(success, _, _)| success)
        .unwrap_or(false);
    
    // Check for remote
    if let Ok((true, remotes, _)) = run_git(path, &["remote"]).await {
        if remotes.is_empty() {
            return (Status::Skip, "no remote".to_string(), has_uncommitted);
        }
    } else {
        return (Status::Skip, "no remote".to_string(), has_uncommitted);
    }
    
    // Get current branch
    let branch = match run_git(path, &["rev-parse", "--abbrev-ref", "HEAD"]).await {
        Ok((true, branch, _)) if branch != "HEAD" => branch,
        _ => return (Status::Skip, "detached HEAD".to_string(), has_uncommitted),
    };
    
    // Check upstream
    if run_git(path, &["rev-parse", "--abbrev-ref", &format!("{}@{{upstream}}", branch)]).await.map(|(s, _, _)| s).unwrap_or(false) == false {
        return (Status::Skip, "no upstream".to_string(), has_uncommitted);
    }
    
    // Fetch
    if let Ok((false, _, err)) = run_git(path, &["fetch", "--quiet"]).await {
        return (Status::Error, format!("fetch failed: {}", err), has_uncommitted);
    }
    
    // Check unpushed commits
    let unpushed = run_git(path, &["rev-list", "--count", &format!("{}@{{upstream}}..HEAD", branch)]).await
        .ok()
        .and_then(|(success, count, _)| if success { count.parse::<u32>().ok() } else { None })
        .unwrap_or(0);
    
    if unpushed > 0 {
        match run_git(path, &["push"]).await {
            Ok((true, _, _)) => (Status::Pushed, format!("{} commits pushed", unpushed), has_uncommitted),
            Ok((false, _, err)) => (Status::Error, format!("push failed: {}", err), has_uncommitted),
            Err(e) => (Status::Error, format!("push error: {}", e), has_uncommitted),
        }
    } else {
        (Status::Synced, "up to date".to_string(), has_uncommitted)
    }
}

fn find_repos() -> Vec<(String, PathBuf)> {
    let mut repos = Vec::new();
    let skip_dirs = ["node_modules", "vendor", "target", "build", ".next", "dist", "__pycache__", ".venv", "venv"];
    
    for entry in WalkDir::new(".").follow_links(true).into_iter().filter_entry(|e| {
        if let Some(file_name) = e.file_name().to_str() {
            !skip_dirs.contains(&file_name)
        } else {
            true
        }
    }) {
        if let Ok(entry) = entry {
            if entry.file_name() == ".git" && entry.file_type().is_dir() {
                if let Some(parent) = entry.path().parent() {
                    let name = if parent == Path::new(".") {
                        // If we're in the current directory, use the directory name
                        if let Ok(current_dir) = std::env::current_dir() {
                            current_dir.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("current")
                                .to_string()
                        } else {
                            "current".to_string()
                        }
                    } else {
                        parent.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown")
                            .to_string()
                    };
                    repos.push((name, parent.to_path_buf()));
                }
            }
        }
    }
    repos
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("🔍 Scanning for git repositories...");
    
    let repos = find_repos();
    if repos.is_empty() {
        println!("No git repositories found in current directory.");
        return Ok(());
    }
    
    let total_repos = repos.len();
    println!("Found {} repositories\n", total_repos);
    
    // Find the maximum repo name length for alignment
    let max_name_len = repos.iter().map(|(name, _)| name.len()).max().unwrap_or(0);
    
    let multi = MultiProgress::new();
    let style = ProgressStyle::default_bar()
        .template("{prefix:.bold} {wide_msg}")?
        .progress_chars("##-");
    
    let stats = Arc::new(Mutex::new((0, 0, 0, 0))); // (synced, commits, skipped, errors)
    
    // Create semaphore to limit to 3 concurrent operations
    let semaphore = Arc::new(tokio::sync::Semaphore::new(3));
    
    // Create futures for all repo operations
    let mut futures = FuturesUnordered::new();
    
    for (name, path) in repos {
        let pb = multi.add(ProgressBar::new(100));
        pb.set_style(style.clone());
        pb.set_prefix(format!("🟡 {}", name));
        pb.set_message("syncing...");
        pb.enable_steady_tick(Duration::from_millis(100));
        
        let stats_clone = Arc::clone(&stats);
        let semaphore_clone = Arc::clone(&semaphore);
        
        let future = async move {
            let _permit = semaphore_clone.acquire().await.unwrap();
            
            let (status, message, has_uncommitted) = check_repo(&path).await;
            
            let display_msg = if has_uncommitted && matches!(status, Status::Synced | Status::Pushed) {
                format!("{} (uncommitted changes)", message)
            } else {
                message.clone()
            };
            
            pb.set_prefix(format!("{} {:width$}", status.symbol(), name, width = max_name_len));
            pb.set_message(format!("{:<10}   {}", status.text(), display_msg));
            pb.finish();
            
            // Update stats
            let mut stats_guard = stats_clone.lock().unwrap();
            match status {
                Status::Pushed => {
                    stats_guard.0 += 1;
                    if let Ok(commits) = message.split_whitespace().next().unwrap_or("0").parse::<u32>() {
                        stats_guard.1 += commits;
                    }
                }
                Status::Synced => stats_guard.0 += 1,
                Status::Skip => stats_guard.2 += 1,
                Status::Error => stats_guard.3 += 1,
            }
        };
        
        futures.push(future);
    }
    
    // Wait for all futures to complete
    while futures.next().await.is_some() {}
    
    let final_stats = *stats.lock().unwrap();
    
    println!();
    let mut summary = format!("Sync complete • {}/{} synced", final_stats.0, total_repos);
    if final_stats.1 > 0 { summary.push_str(&format!(" • {} commits pushed", final_stats.1)); }
    if final_stats.2 > 0 { summary.push_str(&format!(" • {} skipped", final_stats.2)); }
    if final_stats.3 > 0 { summary.push_str(&format!(" • {} errors", final_stats.3.to_string().red())); }
    println!("{}", summary);
    
    Ok(())
}