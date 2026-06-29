use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::graph::{NodeKind, RepoRole, WorkspaceNode};

#[derive(Debug, Clone)]
pub struct InventoryOptions {
    pub root: PathBuf,
    pub include_root: bool,
    pub metadata_file: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceDiscovery {
    pub nodes: BTreeMap<String, WorkspaceNode>,
    pub metadata_path: Option<PathBuf>,
}

pub fn discover_inventory(
    root: &Path,
    metadata_path: Option<&Path>,
) -> Result<WorkspaceDiscovery, String> {
    let mut nodes = BTreeMap::new();

    // Parse .gitmodules
    let gitmodules_path = root.join(".gitmodules");
    if gitmodules_path.exists() {
        let content = std::fs::read_to_string(&gitmodules_path)
            .map_err(|e| format!("Read .gitmodules: {e}"))?;
        for entry in parse_gitmodules(&content) {
            let id = entry.name.clone();
            nodes.insert(
                id.clone(),
                WorkspaceNode {
                    id,
                    path: root.join(&entry.path),
                    repo_url: Some(entry.url),
                    kind: NodeKind::Unknown,
                    role: RepoRole::Unknown,
                    layer: None,
                    is_root: false,
                },
            );
        }
    }

    // Read metadata file for classification
    if let Some(meta_path) = metadata_path {
        let meta_full = if meta_path.is_relative() {
            root.join(meta_path)
        } else {
            meta_path.to_path_buf()
        };
        if meta_full.exists() {
            let content = std::fs::read_to_string(&meta_full)
                .map_err(|e| format!("Read metadata {}: {e}", meta_full.display()))?;

            // Try new topology format first (repos array), fall back to old format (nodes map)
            if content.contains("\"repos\"") {
                apply_topology(&mut nodes, root, &content, &meta_full)?;
            } else {
                apply_metadata(&mut nodes, &content, &meta_full)?;
            }
        }
    }

    // Add root node if not already defined by topology metadata
    if !nodes.values().any(|n| n.is_root) {
        nodes.insert(
            "phenix".to_string(),
            WorkspaceNode {
                id: "phenix".to_string(),
                path: root.to_path_buf(),
                repo_url: None,
                kind: NodeKind::WorkspaceRoot,
                role: RepoRole::Root,
                layer: Some(999),
                is_root: true,
            },
        );
    }

    Ok(WorkspaceDiscovery {
        nodes,
        metadata_path: metadata_path.map(|p| p.to_path_buf()),
    })
}

#[derive(Debug)]
struct GitmoduleEntry {
    name: String,
    path: String,
    url: String,
}

fn parse_gitmodules(content: &str) -> Vec<GitmoduleEntry> {
    let mut entries = Vec::new();
    let mut current: Option<GitmoduleEntry> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(entry) = current.take() {
                entries.push(entry);
            }
            let inner = &line[1..line.len() - 1].trim();
            if let Some(name) = inner.strip_prefix("submodule \"") {
                if let Some(name) = name.strip_suffix('"') {
                    current = Some(GitmoduleEntry {
                        name: name.to_string(),
                        path: String::new(),
                        url: String::new(),
                    });
                }
            }
        } else if let Some(ref mut entry) = current {
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                match key {
                    "path" => entry.path = value.to_string(),
                    "url" => entry.url = value.to_string(),
                    _ => {}
                }
            }
        }
    }

    if let Some(entry) = current {
        entries.push(entry);
    }

    entries
}

fn apply_topology(
    nodes: &mut BTreeMap<String, WorkspaceNode>,
    root: &Path,
    content: &str,
    meta_path: &Path,
) -> Result<(), String> {
    let topology: TopologyFile = serde_json::from_str(content)
        .map_err(|e| format!("Parse topology {}: {e}", meta_path.display()))?;

    for repo in topology.repos {
        let path = if repo.path == "." {
            root.to_path_buf()
        } else {
            root.join(&repo.path)
        };

        let role = repo.role;
        let kind = role_to_kind(role);
        let is_root = role == RepoRole::Root;

        let existing = nodes.get_mut(&repo.name);

        if let Some(node) = existing {
            node.path = path;
            node.kind = kind;
            node.role = role;
            node.layer = Some(repo.layer);
            node.is_root = is_root;
            if repo.url.is_some() {
                node.repo_url = repo.url;
            }
        } else {
            nodes.insert(
                repo.name.clone(),
                WorkspaceNode {
                    id: repo.name,
                    path,
                    repo_url: repo.url,
                    kind,
                    role,
                    layer: Some(repo.layer),
                    is_root,
                },
            );
        }
    }

    Ok(())
}

fn role_to_kind(role: RepoRole) -> NodeKind {
    match role {
        RepoRole::Pins => NodeKind::Pins,
        RepoRole::Lib => NodeKind::Unknown,
        RepoRole::PkgsBase => NodeKind::Unknown,
        RepoRole::Protocols => NodeKind::Unknown,
        RepoRole::Producer => NodeKind::ToolProvider,
        RepoRole::Integration => NodeKind::Unknown,
        RepoRole::PkgsAggregator => NodeKind::PackageProvider,
        RepoRole::Consumer => NodeKind::HostConsumer,
        RepoRole::Root => NodeKind::WorkspaceRoot,
        RepoRole::External => NodeKind::External,
        RepoRole::Unknown => NodeKind::Unknown,
    }
}

fn apply_metadata(
    nodes: &mut BTreeMap<String, WorkspaceNode>,
    content: &str,
    meta_path: &Path,
) -> Result<(), String> {
    let meta: MetadataFile = serde_json::from_str(content)
        .map_err(|e| format!("Parse metadata {}: {e}", meta_path.display()))?;

    for (node_id, meta_node) in &meta.nodes {
        let kind = meta_node.kind();
        let role = kind_to_role(kind);

        if let Some(node) = nodes.get_mut(node_id) {
            node.kind = kind;
            node.role = role;
            node.layer = Some(meta_node.layer);
            if meta_node.root.unwrap_or(false) {
                node.is_root = true;
            }
        } else {
            nodes.insert(
                node_id.clone(),
                WorkspaceNode {
                    id: node_id.clone(),
                    path: PathBuf::new(),
                    repo_url: None,
                    kind,
                    role,
                    layer: Some(meta_node.layer),
                    is_root: meta_node.root.unwrap_or(false),
                },
            );
        }
    }

    Ok(())
}

fn kind_to_role(kind: NodeKind) -> RepoRole {
    match kind {
        NodeKind::Pins => RepoRole::Pins,
        NodeKind::PackageProvider => RepoRole::PkgsAggregator,
        NodeKind::ToolProvider => RepoRole::Producer,
        NodeKind::ShellProvider => RepoRole::Producer,
        NodeKind::DesktopProvider => RepoRole::Consumer,
        NodeKind::HostConsumer => RepoRole::Consumer,
        NodeKind::WorkspaceRoot => RepoRole::Root,
        NodeKind::External => RepoRole::External,
        NodeKind::Unknown => RepoRole::Unknown,
    }
}

// ── New topology format (`.stitch/topology.json`) ──

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TopologyFile {
    version: Option<u32>,
    workspace: Option<String>,
    root: Option<String>,
    repos: Vec<TopologyRepo>,
    rules: Option<TopologyRules>,
}

#[derive(Debug, Deserialize)]
struct TopologyRepo {
    name: String,
    role: RepoRole,
    layer: u32,
    path: String,
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TopologyRules {
    forbid_cycles: Option<bool>,
    forbid_root_dependency: Option<bool>,
    forbid_same_layer_internal_inputs: Option<bool>,
    forbid_producer_depends_on_producer: Option<bool>,
    forbid_producer_depends_on_pkgs_aggregator: Option<bool>,
    require_path_layer_matches_configured_layer: Option<bool>,
}

// ── Old metadata format (`stitch.workspace.json`) ──

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MetadataFile {
    version: u32,
    nodes: BTreeMap<String, MetadataNode>,
    rules: Option<MetadataRules>,
}

#[derive(Debug, Deserialize)]
struct MetadataNode {
    kind: String,
    layer: u32,
    root: Option<bool>,
}

impl MetadataNode {
    fn kind(&self) -> NodeKind {
        match self.kind.as_str() {
            "pins" => NodeKind::Pins,
            "packageProvider" => NodeKind::PackageProvider,
            "toolProvider" => NodeKind::ToolProvider,
            "shellProvider" => NodeKind::ShellProvider,
            "desktopProvider" => NodeKind::DesktopProvider,
            "hostConsumer" => NodeKind::HostConsumer,
            "workspaceRoot" => NodeKind::WorkspaceRoot,
            "external" => NodeKind::External,
            _ => NodeKind::Unknown,
        }
    }
}

#[derive(Debug, Deserialize)]
struct MetadataRules {
    #[allow(dead_code)]
    forbid_cycles: Option<bool>,
    #[allow(dead_code)]
    forbid_root_provider_role: Option<bool>,
    #[allow(dead_code)]
    forbid_provider_depends_on_consumer: Option<bool>,
    #[allow(dead_code)]
    require_root_ultimate_consumer: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gitmodules() {
        let content = r#"
[submodule "phenix-tools"]
  path = flakes/02-producers/phenix-tools
  url = git@github.com:matthis-k/phenix-tools.git
[submodule "phenix-pins"]
  path = flakes/00-pins/phenix-pins
  url = git@github.com:matthis-k/phenix-pins.git
"#;
        let entries = parse_gitmodules(content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "phenix-tools");
        assert_eq!(entries[0].path, "flakes/02-producers/phenix-tools");
        assert_eq!(entries[0].url, "git@github.com:matthis-k/phenix-tools.git");
        assert_eq!(entries[1].name, "phenix-pins");
    }

    #[test]
    fn test_discover_inventory_no_gitmodules() {
        let dir = std::env::temp_dir().join("test_inventory_no_gm");
        let _ = std::fs::create_dir_all(&dir);
        let result = discover_inventory(&dir, None).unwrap();
        assert!(result.nodes.contains_key("phenix"));
        assert_eq!(result.nodes.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_topology_json() {
        let json = r#"{
            "version": 1,
            "workspace": "phenix",
            "repos": [
                {"name": "phenix-pins", "role": "pins", "layer": 0, "path": "flakes/00-pins/phenix-pins"},
                {"name": "phenix-tools", "role": "producer", "layer": 2, "path": "flakes/02-producers/phenix-tools"},
                {"name": "phenix", "role": "root", "layer": 6, "path": "."}
            ]
        }"#;

        let topology: TopologyFile = serde_json::from_str(json).unwrap();
        assert_eq!(topology.repos.len(), 3);
        assert_eq!(topology.repos[0].name, "phenix-pins");
        assert_eq!(topology.repos[0].role, RepoRole::Pins);
        assert_eq!(topology.repos[0].layer, 0);
        assert_eq!(topology.repos[2].name, "phenix");
        assert_eq!(topology.repos[2].role, RepoRole::Root);
    }

    #[test]
    fn test_role_to_kind_roundtrip() {
        assert_eq!(role_to_kind(RepoRole::Pins), NodeKind::Pins);
        assert_eq!(role_to_kind(RepoRole::Producer), NodeKind::ToolProvider);
        assert_eq!(
            role_to_kind(RepoRole::PkgsAggregator),
            NodeKind::PackageProvider
        );
        assert_eq!(role_to_kind(RepoRole::Consumer), NodeKind::HostConsumer);
        assert_eq!(role_to_kind(RepoRole::Root), NodeKind::WorkspaceRoot);
        assert_eq!(role_to_kind(RepoRole::External), NodeKind::External);
    }

    #[test]
    fn test_kind_to_role_roundtrip() {
        assert_eq!(kind_to_role(NodeKind::Pins), RepoRole::Pins);
        assert_eq!(kind_to_role(NodeKind::ToolProvider), RepoRole::Producer);
        assert_eq!(
            kind_to_role(NodeKind::PackageProvider),
            RepoRole::PkgsAggregator
        );
        assert_eq!(kind_to_role(NodeKind::HostConsumer), RepoRole::Consumer);
        assert_eq!(kind_to_role(NodeKind::WorkspaceRoot), RepoRole::Root);
        assert_eq!(kind_to_role(NodeKind::External), RepoRole::External);
    }
}
