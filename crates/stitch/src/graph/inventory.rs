use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::graph::{NodeKind, RepoRole, WorkspaceNode};
use crate::model::WorkspaceConfig;

#[derive(Debug, Clone)]
pub struct WorkspaceDiscovery {
    pub nodes: BTreeMap<String, WorkspaceNode>,
    pub metadata_path: Option<PathBuf>,
}

/// Discover workspace members through Stitch's workspace policy, then apply
/// optional classification metadata. Membership is never created by metadata.
pub fn discover_inventory(
    root: &Path,
    metadata_path: Option<&Path>,
) -> Result<WorkspaceDiscovery, String> {
    let cfg = crate::workspace::load_workspace_config(root)?;
    discover_inventory_from_config(&cfg, metadata_path)
}

pub fn discover_inventory_from_config(
    cfg: &WorkspaceConfig,
    metadata_path: Option<&Path>,
) -> Result<WorkspaceDiscovery, String> {
    let root = cfg.config_dir.as_deref().ok_or_else(|| {
        "Cannot discover workspace inventory: workspace root is unavailable".to_string()
    })?;
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());

    let mut nodes = BTreeMap::new();
    for repo in &cfg.repos {
        let path = repo.resolved_path(cfg);
        let canonical_path = path.canonicalize().unwrap_or(path);
        let is_root = repo.name == cfg.workspace || canonical_path == canonical_root;
        let (kind, role) = if is_root {
            (NodeKind::WorkspaceRoot, RepoRole::Root)
        } else {
            (NodeKind::Unknown, RepoRole::Unknown)
        };

        if nodes
            .insert(
                repo.name.clone(),
                WorkspaceNode {
                    id: repo.name.clone(),
                    path: canonical_path,
                    repo_url: repo.remote.clone(),
                    kind,
                    role,
                    layer: None,
                    is_root,
                },
            )
            .is_some()
        {
            return Err(format!(
                "Workspace inventory contains duplicate repository id '{}'",
                repo.name
            ));
        }
    }

    if nodes.is_empty() {
        return Err("Workspace discovery produced no repositories".to_string());
    }
    let roots = nodes.values().filter(|node| node.is_root).count();
    if roots != 1 {
        return Err(format!(
            "Workspace inventory must contain exactly one root repository, found {roots}"
        ));
    }

    let metadata_path = resolve_metadata_path(root, metadata_path);
    if let Some(path) = metadata_path.as_deref() {
        apply_classification_metadata(&mut nodes, path)?;
    }

    Ok(WorkspaceDiscovery {
        nodes,
        metadata_path,
    })
}

fn resolve_metadata_path(root: &Path, metadata_path: Option<&Path>) -> Option<PathBuf> {
    let path = metadata_path?;
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    resolved.exists().then_some(resolved)
}

fn apply_classification_metadata(
    nodes: &mut BTreeMap<String, WorkspaceNode>,
    metadata_path: &Path,
) -> Result<(), String> {
    let content = std::fs::read_to_string(metadata_path)
        .map_err(|e| format!("Read metadata {}: {e}", metadata_path.display()))?;
    let metadata: WorkspaceMetadata = serde_json::from_str(&content)
        .map_err(|e| format!("Parse metadata {}: {e}", metadata_path.display()))?;

    let mut declared_root = None;
    for repo in metadata.repos {
        let Some(node) = nodes.get_mut(&repo.name) else {
            return Err(format!(
                "Metadata {} references non-member repository '{}'",
                metadata_path.display(),
                repo.name
            ));
        };

        node.role = repo.role;
        node.kind = role_to_kind(repo.role);
        node.layer = Some(repo.layer);
        if repo.role == RepoRole::Root {
            if let Some(existing) = &declared_root {
                return Err(format!(
                    "Metadata {} declares multiple roots: '{}' and '{}'",
                    metadata_path.display(),
                    existing,
                    repo.name
                ));
            }
            declared_root = Some(repo.name.clone());
        }
    }

    if let Some(root_id) = declared_root {
        for node in nodes.values_mut() {
            node.is_root = node.id == root_id;
            if node.is_root {
                node.kind = NodeKind::WorkspaceRoot;
                node.role = RepoRole::Root;
            }
        }
    }

    Ok(())
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
struct WorkspaceMetadata {
    #[serde(default)]
    workspace: Option<String>,
    repos: Vec<WorkspaceMetadataRepo>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceMetadataRepo {
    name: String,
    role: RepoRole,
    layer: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RepoConfig;

    fn config(root: &Path) -> WorkspaceConfig {
        WorkspaceConfig {
            workspace: "workspace".to_string(),
            repos: vec![
                RepoConfig {
                    name: "workspace".to_string(),
                    path: ".".to_string(),
                    remote: None,
                },
                RepoConfig {
                    name: "phenix-stitch".to_string(),
                    path: "repos/phenix-stitch".to_string(),
                    remote: Some("github:matthis-k/phenix-stitch".to_string()),
                },
            ],
            config_dir: Some(root.to_path_buf()),
        }
    }

    #[test]
    fn workspace_config_is_the_membership_authority() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repos/phenix-stitch")).unwrap();

        let discovery = discover_inventory_from_config(&config(dir.path()), None).unwrap();

        assert_eq!(discovery.nodes.len(), 2);
        assert!(discovery.nodes["workspace"].is_root);
        assert_eq!(
            discovery.nodes["phenix-stitch"].repo_url.as_deref(),
            Some("github:matthis-k/phenix-stitch")
        );
    }

    #[test]
    fn metadata_only_classifies_existing_members() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repos/phenix-stitch")).unwrap();
        let metadata = dir.path().join("metadata.json");
        std::fs::write(
            &metadata,
            r#"{
                "repos": [
                    {"name": "workspace", "role": "root", "layer": 6},
                    {"name": "phenix-stitch", "role": "producer", "layer": 2}
                ]
            }"#,
        )
        .unwrap();

        let discovery =
            discover_inventory_from_config(&config(dir.path()), Some(&metadata)).unwrap();

        assert_eq!(discovery.nodes["phenix-stitch"].role, RepoRole::Producer);
        assert_eq!(discovery.nodes["phenix-stitch"].layer, Some(2));
        assert_eq!(discovery.nodes.len(), 2);
    }

    #[test]
    fn metadata_cannot_add_workspace_members() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("repos/phenix-stitch")).unwrap();
        let metadata = dir.path().join("metadata.json");
        std::fs::write(
            &metadata,
            r#"{
                "repos": [
                    {"name": "not-discovered", "role": "producer", "layer": 2}
                ]
            }"#,
        )
        .unwrap();

        let error = discover_inventory_from_config(&config(dir.path()), Some(&metadata))
            .err()
            .unwrap();
        assert!(error.contains("non-member repository 'not-discovered'"));
    }
}
