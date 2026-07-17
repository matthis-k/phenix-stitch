use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

use crate::graph::{EdgeKind, EdgeSpec, NodeKind, NodeSpec, RepoRole};
use crate::model::WorkspaceConfig;

#[derive(Debug, Clone)]
pub struct WorkspaceDiscovery {
    pub nodes: BTreeMap<String, NodeSpec>,
    pub manual_edges: Vec<EdgeSpec>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepositoryGraphConfig {
    #[serde(default)]
    role: Option<RepoRole>,
    #[serde(default)]
    layer: Option<u32>,
    #[serde(default)]
    dependencies: Vec<String>,
}

pub fn discover_inventory(
    root: &Path,
    _legacy_metadata: Option<&Path>,
) -> Result<WorkspaceDiscovery, String> {
    let config = crate::workspace::load_workspace_config(root)?;
    discover_inventory_from_config(&config, None)
}

pub fn discover_inventory_from_config(
    config: &WorkspaceConfig,
    _legacy_metadata: Option<&Path>,
) -> Result<WorkspaceDiscovery, String> {
    let root = config.config_dir.as_deref().ok_or_else(|| {
        "Cannot discover workspace inventory: workspace root is unavailable".to_string()
    })?;
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut nodes = BTreeMap::new();
    let mut declarations = BTreeMap::new();

    for repository in &config.repos {
        let path = repository.resolved_path(config);
        let canonical_path = path.canonicalize().unwrap_or(path);
        let is_root = repository.name == config.workspace || canonical_path == canonical_root;
        let declaration = load_repository_graph_config(&canonical_path)?;
        let role = if is_root {
            RepoRole::Root
        } else {
            declaration.role.unwrap_or(RepoRole::Unknown)
        };
        let kind = role_to_kind(role);
        let layer = declaration.layer.or_else(|| role.layer());

        let node = NodeSpec {
            id: repository.name.clone(),
            path: canonical_path,
            repo_url: repository.remote.clone(),
            kind,
            role,
            layer,
            is_root,
        };
        if nodes.insert(repository.name.clone(), node).is_some() {
            return Err(format!(
                "Workspace inventory contains duplicate repository id '{}'",
                repository.name
            ));
        }
        declarations.insert(repository.name.clone(), declaration);
    }

    if nodes.values().filter(|node| node.is_root).count() != 1 {
        return Err("Workspace inventory must contain exactly one root repository".to_string());
    }

    let mut manual_edges = Vec::new();
    for (source, declaration) in declarations {
        let source_file = nodes[&source].path.join(".stitch.json");
        for target in declaration.dependencies {
            if !nodes.contains_key(&target) {
                return Err(format!(
                    "{} declares unknown workspace dependency '{}'",
                    source_file.display(),
                    target
                ));
            }
            if source != target {
                manual_edges.push(EdgeSpec {
                    from: source.clone(),
                    to: target,
                    kind: EdgeKind::Manual {
                        source_file: source_file.clone(),
                    },
                });
            }
        }
    }

    Ok(WorkspaceDiscovery {
        nodes,
        manual_edges,
    })
}

fn load_repository_graph_config(path: &Path) -> Result<RepositoryGraphConfig, String> {
    let file = path.join(".stitch.json");
    if !file.exists() {
        return Ok(RepositoryGraphConfig::default());
    }
    let content = std::fs::read_to_string(&file)
        .map_err(|error| format!("Read {}: {error}", file.display()))?;
    serde_json::from_str(&content).map_err(|error| format!("Parse {}: {error}", file.display()))
}

fn role_to_kind(role: RepoRole) -> NodeKind {
    match role {
        RepoRole::Pins => NodeKind::Pins,
        RepoRole::PkgsAggregator => NodeKind::PackageProvider,
        RepoRole::Producer => NodeKind::ToolProvider,
        RepoRole::Consumer => NodeKind::HostConsumer,
        RepoRole::Root => NodeKind::WorkspaceRoot,
        RepoRole::External => NodeKind::External,
        RepoRole::Lib
        | RepoRole::PkgsBase
        | RepoRole::Protocols
        | RepoRole::Integration
        | RepoRole::Unknown => NodeKind::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RepoConfig;

    #[test]
    fn repository_local_config_adds_classification_and_edges() {
        let directory = tempfile::tempdir().unwrap();
        let provider = directory.path().join("repos/provider");
        std::fs::create_dir_all(&provider).unwrap();
        std::fs::write(
            provider.join(".stitch.json"),
            r#"{"role":"producer","layer":2,"dependencies":["workspace"]}"#,
        )
        .unwrap();
        let config = WorkspaceConfig {
            workspace: "workspace".to_string(),
            repos: vec![
                RepoConfig {
                    name: "workspace".to_string(),
                    path: ".".to_string(),
                    remote: None,
                },
                RepoConfig {
                    name: "provider".to_string(),
                    path: "repos/provider".to_string(),
                    remote: None,
                },
            ],
            config_dir: Some(directory.path().to_path_buf()),
        };
        let discovery = discover_inventory_from_config(&config, None).unwrap();
        assert_eq!(discovery.nodes["provider"].role, RepoRole::Producer);
        assert_eq!(discovery.manual_edges.len(), 1);
    }
}
