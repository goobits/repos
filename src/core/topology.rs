//! Dependency-aware ordering for repositories discovered in one filesystem tree.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Direction in which repository mutation waves should run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RepositoryOrder {
    ParentsFirst,
    ChildrenFirst,
}

#[derive(Clone, Debug)]
pub(crate) struct GitlinkPrerequisite {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) target: String,
}

/// Filesystem nesting plus the subset of relationships backed by Git gitlinks.
///
/// Ordinary nested repositories are ordered for predictable fleet behavior.
/// Only registered submodules are hard dependencies that may block a parent.
#[derive(Debug)]
pub(crate) struct RepositoryTopology {
    levels: Vec<usize>,
    gitlink_children: Vec<Vec<usize>>,
    gitlink_targets: HashMap<(usize, usize), String>,
}

impl RepositoryTopology {
    pub(crate) fn new(repositories: &[(String, PathBuf)]) -> Self {
        let normalized = repositories
            .iter()
            .map(|(_, path)| normalize_path(path))
            .collect::<Vec<_>>();
        let mut parents = vec![None; repositories.len()];

        for child in 0..repositories.len() {
            parents[child] = (0..repositories.len())
                .filter(|parent| {
                    *parent != child
                        && normalized[child].starts_with(&normalized[*parent])
                        && normalized[child] != normalized[*parent]
                })
                .max_by_key(|parent| normalized[*parent].components().count());
        }

        let levels = (0..repositories.len())
            .map(|index| repository_level(index, &parents))
            .collect::<Vec<_>>();
        let mut gitlink_children = vec![Vec::new(); repositories.len()];
        let mut gitlink_targets = HashMap::new();
        for (child, parent) in parents.iter().enumerate() {
            let Some(parent) = parent else {
                continue;
            };
            if let Some(target) = gitlink_target(
                &repositories[*parent].1,
                &normalized[*parent],
                &normalized[child],
            ) {
                gitlink_children[*parent].push(child);
                gitlink_targets.insert((*parent, child), target);
            }
        }

        Self {
            levels,
            gitlink_children,
            gitlink_targets,
        }
    }

    /// Returns independent waves while preserving the original order within a wave.
    pub(crate) fn waves(&self, order: RepositoryOrder) -> Vec<Vec<usize>> {
        let Some(max_level) = self.levels.iter().copied().max() else {
            return Vec::new();
        };
        let levels: Box<dyn Iterator<Item = usize>> = match order {
            RepositoryOrder::ParentsFirst => Box::new(0..=max_level),
            RepositoryOrder::ChildrenFirst => Box::new((0..=max_level).rev()),
        };

        levels
            .map(|level| {
                self.levels
                    .iter()
                    .enumerate()
                    .filter_map(|(index, candidate)| (*candidate == level).then_some(index))
                    .collect::<Vec<_>>()
            })
            .filter(|wave| !wave.is_empty())
            .collect()
    }

    pub(crate) fn gitlink_children(&self, parent: usize) -> &[usize] {
        &self.gitlink_children[parent]
    }

    pub(crate) fn gitlink_target(&self, parent: usize, child: usize) -> Option<&str> {
        self.gitlink_targets
            .get(&(parent, child))
            .map(String::as_str)
    }

    pub(crate) fn gitlink_prerequisites(
        &self,
        parent: usize,
        repositories: &[(String, PathBuf)],
    ) -> Vec<GitlinkPrerequisite> {
        self.gitlink_children(parent)
            .iter()
            .filter_map(|child| {
                self.gitlink_target(parent, *child)
                    .map(|target| GitlinkPrerequisite {
                        name: repositories[*child].0.clone(),
                        path: repositories[*child].1.clone(),
                        target: target.to_string(),
                    })
            })
            .collect()
    }

    pub(crate) fn has_gitlink_dependencies(&self) -> bool {
        self.gitlink_children
            .iter()
            .any(|children| !children.is_empty())
    }
}

pub(crate) fn normalize_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path)
        }
    })
}

fn repository_level(index: usize, parents: &[Option<usize>]) -> usize {
    let mut level = 0;
    let mut current = parents[index];
    while let Some(parent) = current {
        level += 1;
        current = parents[parent];
    }
    level
}

pub(crate) fn gitlink_target(
    parent_path: &Path,
    normalized_parent: &Path,
    normalized_child: &Path,
) -> Option<String> {
    let Ok(relative) = normalized_child.strip_prefix(normalized_parent) else {
        return None;
    };
    let output = Command::new("git")
        .arg("-C")
        .arg(parent_path)
        .args(["ls-files", "--stage", "--"])
        .arg(relative)
        .output();

    output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            output_gitlink_target(&String::from_utf8_lossy(&output.stdout), relative)
        })
}

fn output_gitlink_target(output: &str, relative: &Path) -> Option<String> {
    let expected = relative.to_string_lossy();
    output.lines().find_map(|line| {
        let (metadata, path) = line.split_once('\t')?;
        let mut fields = metadata.split_whitespace();
        (fields.next() == Some("160000") && path == expected)
            .then(|| fields.next().map(str::to_string))
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names_for_waves(
        repositories: &[(String, PathBuf)],
        waves: Vec<Vec<usize>>,
    ) -> Vec<Vec<String>> {
        waves
            .into_iter()
            .map(|wave| {
                wave.into_iter()
                    .map(|index| repositories[index].0.clone())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn orders_nested_repositories_in_dependency_waves() {
        let root = std::env::current_dir().unwrap().join("topology-fixture");
        let repositories = vec![
            ("parent".to_string(), root.clone()),
            ("sibling".to_string(), root.join("sibling")),
            ("child".to_string(), root.join("child")),
            ("grandchild".to_string(), root.join("child/grandchild")),
        ];
        let topology = RepositoryTopology::new(&repositories);

        assert_eq!(
            names_for_waves(
                &repositories,
                topology.waves(RepositoryOrder::ChildrenFirst)
            ),
            vec![
                vec!["grandchild".to_string()],
                vec!["sibling".to_string(), "child".to_string()],
                vec!["parent".to_string()],
            ]
        );
        assert_eq!(
            names_for_waves(&repositories, topology.waves(RepositoryOrder::ParentsFirst)),
            vec![
                vec!["parent".to_string()],
                vec!["sibling".to_string(), "child".to_string()],
                vec!["grandchild".to_string()],
            ]
        );
    }

    #[test]
    fn recognizes_only_the_exact_gitlink_path() {
        let output = "160000 abcdef 0\tpackages/shared\n100644 fedcba 0\tpackages/other\n";

        assert_eq!(
            output_gitlink_target(output, Path::new("packages/shared")).as_deref(),
            Some("abcdef")
        );
        assert!(output_gitlink_target(output, Path::new("packages")).is_none());
        assert!(output_gitlink_target(output, Path::new("packages/other")).is_none());
    }
}
