//! Dependency-aware ordering for repositories discovered in one filesystem tree.

use std::collections::HashMap;
use std::ffi::OsString;
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

/// Canonical filesystem nesting for one immutable discovery snapshot.
#[derive(Debug)]
pub(crate) struct FleetIndex {
    normalized_paths: Vec<PathBuf>,
    repositories_by_path: HashMap<PathBuf, usize>,
    parents: Vec<Option<usize>>,
    levels: Vec<usize>,
}

/// Filesystem and gitlink topology derived once from a discovery snapshot.
#[derive(Debug)]
pub(crate) struct TopologySnapshot {
    index: FleetIndex,
    topology: RepositoryTopology,
}

impl TopologySnapshot {
    pub(crate) fn new(repositories: &[(String, PathBuf)]) -> Self {
        let index = FleetIndex::new(repositories);
        let topology = RepositoryTopology::from_index(repositories, &index);
        Self { index, topology }
    }

    pub(crate) fn index(&self) -> &FleetIndex {
        &self.index
    }

    pub(crate) fn topology(&self) -> &RepositoryTopology {
        &self.topology
    }
}

impl FleetIndex {
    pub(crate) fn new(repositories: &[(String, PathBuf)]) -> Self {
        let normalized_paths = repositories
            .iter()
            .map(|(_, path)| normalize_path(path))
            .collect::<Vec<_>>();
        let repositories_by_path = normalized_paths
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, path)| (path, index))
            .collect::<HashMap<_, _>>();
        let parents = normalized_paths
            .iter()
            .enumerate()
            .map(|(child, path)| {
                path.ancestors()
                    .skip(1)
                    .find_map(|ancestor| repositories_by_path.get(ancestor).copied())
                    .filter(|parent| *parent != child)
            })
            .collect::<Vec<_>>();
        let levels = (0..repositories.len())
            .map(|index| repository_level(index, &parents))
            .collect();

        Self {
            normalized_paths,
            repositories_by_path,
            parents,
            levels,
        }
    }

    pub(crate) fn parent(&self, child: usize) -> Option<usize> {
        self.parents[child]
    }

    pub(crate) fn normalized_path(&self, index: usize) -> &Path {
        &self.normalized_paths[index]
    }

    pub(crate) fn repository_at(&self, path: &Path) -> Option<usize> {
        self.repositories_by_path.get(path).copied()
    }
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
    indexed_gitlinks: Vec<HashMap<PathBuf, String>>,
    gitlink_errors: Vec<Option<String>>,
}

impl RepositoryTopology {
    pub(crate) fn new(repositories: &[(String, PathBuf)]) -> Self {
        Self::from_index(repositories, &FleetIndex::new(repositories))
    }

    pub(crate) fn from_index(repositories: &[(String, PathBuf)], index: &FleetIndex) -> Self {
        let mut gitlink_children = vec![Vec::new(); repositories.len()];
        let mut gitlink_targets = HashMap::new();
        let mut indexed_gitlinks_by_parent = vec![HashMap::new(); repositories.len()];
        let mut gitlink_errors = vec![None; repositories.len()];

        let mut children_by_parent = vec![Vec::new(); repositories.len()];
        for child in 0..repositories.len() {
            if let Some(parent) = index.parent(child) {
                children_by_parent[parent].push(child);
            }
        }
        for (parent, children) in children_by_parent.into_iter().enumerate() {
            if children.is_empty() && !repositories[parent].1.join(".gitmodules").is_file() {
                continue;
            }
            let parent_gitlinks = match indexed_gitlinks(&repositories[parent].1) {
                Ok(gitlinks) => gitlinks,
                Err(error) => {
                    gitlink_errors[parent] = Some(error);
                    continue;
                }
            };
            indexed_gitlinks_by_parent[parent] = parent_gitlinks.clone();
            for child in children {
                let Ok(relative) = index
                    .normalized_path(child)
                    .strip_prefix(index.normalized_path(parent))
                else {
                    continue;
                };
                if let Some(target) = parent_gitlinks.get(relative) {
                    gitlink_children[parent].push(child);
                    gitlink_targets.insert((parent, child), target.clone());
                }
            }
        }

        Self {
            levels: index.levels.clone(),
            gitlink_children,
            gitlink_targets,
            indexed_gitlinks: indexed_gitlinks_by_parent,
            gitlink_errors,
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

    pub(crate) fn is_gitlink(&self, parent: usize, child: usize) -> bool {
        self.gitlink_targets.contains_key(&(parent, child))
    }

    pub(crate) fn gitlink_inspection_error(&self, parent: usize) -> Option<&str> {
        self.gitlink_errors[parent].as_deref()
    }

    pub(crate) fn indexed_gitlinks(&self, parent: usize) -> &HashMap<PathBuf, String> {
        &self.indexed_gitlinks[parent]
    }

    pub(crate) fn has_gitlink_inspection_failures(&self) -> bool {
        self.gitlink_errors.iter().any(Option::is_some)
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

/// Re-resolves known child gitlinks from the exact parent commit that would be pushed.
pub(crate) fn gitlink_prerequisites_at_head(
    parent_path: &Path,
    prerequisites: &[GitlinkPrerequisite],
) -> Result<Vec<GitlinkPrerequisite>, String> {
    let gitlinks = head_gitlinks(parent_path)?;
    let normalized_parent = normalize_path(parent_path);

    prerequisites
        .iter()
        .filter_map(|prerequisite| {
            let normalized_child = normalize_path(&prerequisite.path);
            let relative = match normalized_child.strip_prefix(&normalized_parent) {
                Ok(relative) => relative,
                Err(_) => {
                    return Some(Err(format!(
                        "submodule path is outside its parent: {}",
                        prerequisite.path.display()
                    )))
                }
            };
            gitlinks.get(relative).map(|target| {
                Ok(GitlinkPrerequisite {
                    name: prerequisite.name.clone(),
                    path: prerequisite.path.clone(),
                    target: target.clone(),
                })
            })
        })
        .collect()
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

fn indexed_gitlinks(parent_path: &Path) -> Result<HashMap<PathBuf, String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(parent_path)
        .args(["ls-files", "--stage", "-z"])
        .output()
        .map_err(|error| format!("failed to run git ls-files: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "git ls-files failed".to_string()
        } else {
            stderr
        });
    }
    Ok(parse_gitlinks(&output.stdout))
}

fn head_gitlinks(parent_path: &Path) -> Result<HashMap<PathBuf, String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(parent_path)
        .args(["ls-tree", "-rz", "--full-tree", "HEAD"])
        .output()
        .map_err(|error| format!("failed to run git ls-tree: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "git ls-tree failed".to_string()
        } else {
            stderr
        });
    }
    Ok(parse_tree_gitlinks(&output.stdout))
}

fn parse_gitlinks(output: &[u8]) -> HashMap<PathBuf, String> {
    output
        .split(|byte| *byte == 0)
        .filter_map(|record| {
            let tab = record.iter().position(|byte| *byte == b'\t')?;
            let metadata = std::str::from_utf8(&record[..tab]).ok()?;
            let mut fields = metadata.split_whitespace();
            if fields.next() != Some("160000") {
                return None;
            }
            let target = fields.next()?.to_string();
            let path = path_from_git_bytes(&record[tab + 1..]);
            Some((path, target))
        })
        .collect()
}

fn parse_tree_gitlinks(output: &[u8]) -> HashMap<PathBuf, String> {
    output
        .split(|byte| *byte == 0)
        .filter_map(|record| {
            let tab = record.iter().position(|byte| *byte == b'\t')?;
            let metadata = std::str::from_utf8(&record[..tab]).ok()?;
            let mut fields = metadata.split_whitespace();
            if fields.next() != Some("160000") || fields.next() != Some("commit") {
                return None;
            }
            let target = fields.next()?.to_string();
            let path = path_from_git_bytes(&record[tab + 1..]);
            Some((path, target))
        })
        .collect()
}

#[cfg(unix)]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).as_ref())
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
        let output = b"160000 abcdef 0\tpackages/shared\0\
              100644 fedcba 0\tpackages/other\0";
        let gitlinks = parse_gitlinks(output);

        assert_eq!(
            gitlinks
                .get(Path::new("packages/shared"))
                .map(String::as_str),
            Some("abcdef")
        );
        assert!(!gitlinks.contains_key(Path::new("packages")));
        assert!(!gitlinks.contains_key(Path::new("packages/other")));
    }

    #[test]
    fn parses_committed_gitlink_targets_from_tree_output() {
        let output = b"160000 commit abcdef\tpackages/shared module\0\
              100644 blob fedcba\tpackages/other\0";
        let gitlinks = parse_tree_gitlinks(output);

        assert_eq!(
            gitlinks
                .get(Path::new("packages/shared module"))
                .map(String::as_str),
            Some("abcdef")
        );
        assert!(!gitlinks.contains_key(Path::new("packages/other")));
    }

    #[test]
    fn fleet_index_finds_the_nearest_parent_independent_of_input_order() {
        let root = std::env::current_dir().unwrap().join("fleet-index-fixture");
        let repositories = vec![
            ("grandchild".to_string(), root.join("child/grandchild")),
            ("parent".to_string(), root.clone()),
            ("child".to_string(), root.join("child")),
        ];
        let index = FleetIndex::new(&repositories);

        assert_eq!(index.parent(0), Some(2));
        assert_eq!(index.parent(2), Some(1));
        assert_eq!(index.parent(1), None);
    }
}
