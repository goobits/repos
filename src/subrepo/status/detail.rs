//! Detailed per-group terminal rendering and sync guidance.

use super::{instance_location, select_sync_target, SubrepoInstance, SubrepoStatus};
use std::collections::HashMap;

#[derive(Debug, PartialEq)]
enum UncommittedState {
    AllClean,
    AllDirty,
    Mixed,
}

pub(super) fn display_drift_status(status: &SubrepoStatus) {
    println!("{}", status.name);
    println!("  Remote: {}", status.remote_url);
    println!(
        "  Sync Score: {}% ({} commits across {} repos)",
        status.sync_score as u32,
        status.unique_commits,
        status.instances.len()
    );
    println!();

    let Some(sync_target) = select_sync_target(status) else {
        return;
    };
    let sync_target_hash = &sync_target.commit_hash;
    let mut by_commit: HashMap<String, Vec<&SubrepoInstance>> = HashMap::new();
    for instance in &status.instances {
        by_commit
            .entry(instance.commit_hash.clone())
            .or_default()
            .push(instance);
    }

    let mut commits = by_commit.into_iter().collect::<Vec<_>>();
    commits.sort_by(|left, right| {
        right
            .1
            .len()
            .cmp(&left.1.len())
            .then_with(|| {
                let left_timestamp = left
                    .1
                    .iter()
                    .map(|instance| instance.commit_timestamp)
                    .max()
                    .unwrap_or(0);
                let right_timestamp = right
                    .1
                    .iter()
                    .map(|instance| instance.commit_timestamp)
                    .max()
                    .unwrap_or(0);
                right_timestamp.cmp(&left_timestamp)
            })
            .then_with(|| left.0.cmp(&right.0))
    });

    for (_, instances) in &mut commits {
        instances.sort_by_key(|instance| instance_location(instance));
        for instance in instances {
            let is_target = instance.commit_hash == *sync_target_hash;
            let prefix = if is_target { "→" } else { " " };
            let worktree = if instance.has_uncommitted {
                "⚠️ uncommitted"
            } else {
                "✅ clean"
            };
            let commit = if is_target {
                "  ⬆️ SYNC TARGET"
            } else {
                "  (different commit)"
            };
            println!(
                "  {} {}  {:width$}  {}{}  {}",
                prefix,
                instance.short_hash,
                instance_location(instance),
                worktree,
                commit,
                instance.checkout_kind.label(),
                width = 30
            );
        }
    }
    println!();
    display_sync_guidance(status);
    println!();
}

fn display_sync_guidance(status: &SubrepoStatus) {
    let Some(target) = select_sync_target(status) else {
        return;
    };
    let dirty_repositories = status
        .instances
        .iter()
        .filter(|instance| instance.has_uncommitted)
        .map(instance_location)
        .collect::<Vec<_>>();

    match analyze_uncommitted_state(&status.instances) {
        UncommittedState::AllDirty => {
            let repositories = if dirty_repositories.len() == 1 {
                format!("'{}'", dirty_repositories[0])
            } else {
                format!("{} repos", dirty_repositories.len())
            };
            println!("  ⚠️ All instances have uncommitted changes.\n");
            println!("  💡 EASY FIX (Recommended):");
            println!(
                "     repos nested sync {} --to {} --stash",
                status.name, target.short_hash
            );
            println!("     (Stashes uncommitted changes in {repositories})\n");
            println!("  Manual alternative: commit or discard local changes, then rerun sync.");
        }
        UncommittedState::Mixed => {
            let dirty_list = dirty_repositories
                .iter()
                .map(|repository| format!("'{repository}'"))
                .collect::<Vec<_>>()
                .join(", ");
            println!("  💡 EASY FIX (Recommended):");
            println!(
                "     repos nested sync {} --to {} --stash",
                status.name, target.short_hash
            );
            println!(
                "     (Syncs {dirty_list} to the clean commit from '{}')\n",
                instance_location(target)
            );
            println!(
                "  Manual alternative: commit or discard changes in {dirty_list}, then rerun sync."
            );
        }
        UncommittedState::AllClean => {
            println!("  🔧 SYNC to selected target:");
            println!(
                "     repos nested sync {} --to {}",
                status.name, target.short_hash
            );
        }
    }
}

fn analyze_uncommitted_state(instances: &[SubrepoInstance]) -> UncommittedState {
    let dirty = instances
        .iter()
        .filter(|instance| instance.has_uncommitted)
        .count();
    if dirty == 0 {
        UncommittedState::AllClean
    } else if dirty == instances.len() {
        UncommittedState::AllDirty
    } else {
        UncommittedState::Mixed
    }
}

pub(super) fn display_synced_status(status: &SubrepoStatus) {
    let Some(commit) = status.instances.first() else {
        return;
    };
    println!("{}", status.name);
    println!("  Remote: {}", status.remote_url);
    println!("  Sync Score: 100% (all at same commit)\n");
    println!("  {}  (all instances)\n", commit.short_hash);

    let has_dirty = status
        .instances
        .iter()
        .any(|instance| instance.has_uncommitted);
    for instance in &status.instances {
        if has_dirty {
            let worktree = if instance.has_uncommitted {
                "⚠️  uncommitted"
            } else {
                "✅ clean"
            };
            println!(
                "    {:width$}  {}  {}",
                instance_location(instance),
                worktree,
                instance.checkout_kind.label(),
                width = 30
            );
        } else {
            println!(
                "    • {}  {}",
                instance_location(instance),
                instance.checkout_kind.label()
            );
        }
    }
    println!();
    if has_dirty {
        println!("  ✅ Already synchronized");
        println!("  ⚠️  But some have uncommitted changes");
    } else {
        println!("  ✅ Already synchronized and clean");
    }
    println!();
}

pub(super) fn display_unique_status(status: &SubrepoStatus) {
    let Some(instance) = status.instances.first() else {
        return;
    };
    let state = if instance.has_uncommitted {
        "⚠️  uncommitted changes"
    } else {
        "✅ clean"
    };

    println!("{}", status.name);
    println!("  Remote: {}", status.remote_url);
    println!(
        "  {} @ {}  {}  {}",
        instance_location(instance),
        instance.short_hash,
        state,
        instance.checkout_kind.label()
    );
    println!("  ℹ️  One discovered copy; cross-copy drift is not applicable\n");
}

pub(super) fn display_missing_remote(instance: &SubrepoInstance) {
    let state = if instance.has_uncommitted {
        " + uncommitted changes"
    } else {
        ""
    };
    println!(
        "{} @ {}  no origin{}  {}",
        instance_location(instance),
        instance.short_hash,
        state,
        instance.checkout_kind.label()
    );
    println!("  ↳ next: add an origin remote or exclude this repository\n");
}
