use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::git;
use crate::model::WorkspaceConfig;

pub mod canonical;
pub mod derive;
pub mod inventory;
pub mod lock;
pub mod planner;
pub mod render;
pub mod spec;
pub mod strategy;
pub mod topo;
pub mod validate;

pub use canonical::{CanonicalWorkspaceGraph, CanonicalizeError};
pub use derive::{derive_workspace_graph, derive_workspace_graph_from_config};
pub use inventory::{discover_inventory, discover_inventory_from_config, WorkspaceDiscovery};
pub use lock::parse_flake_lock;
pub use planner::{
    DagPlan, DagPlanRequest, DagPlanner, PlanClosureMode, PlanOrderMode, PlanSelectionMode,
    PlannedDagNode,
};
pub use render::RenderFormat;
pub use spec::{
    DagGenerationStrategy, EdgeKind, EdgeSpec, GenerationContext, NodeSpec, StrategyError,
    WorkspaceGraphDraft,
};
pub use strategy::{CompositeDagGenerationStrategy, FlakeLocksStrategy, GitSubmodulesStrategy};
pub use topo::provider_before_consumer_order;
pub use validate::{
    validate_canonical_graph, validate_graph, DiagnosticSeverity, GraphDiagnostic,
    GraphValidationReport, ValidateOptions,
};

// ---------------------------------------------------------------------------
// Lock-derived graph types (new subsystem)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphSource {
    Locks,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Pins,
    PackageProvider,
    ToolProvider,
    ShellProvider,
    DesktopProvider,
    HostConsumer,
    WorkspaceRoot,
    External,
    Unknown,
}

impl NodeKind {
    pub fn is_provider(&self) -> bool {
        matches!(
            self,
            NodeKind::Pins
                | NodeKind::PackageProvider
                | NodeKind::ToolProvider
                | NodeKind::ShellProvider
                | NodeKind::DesktopProvider
        )
    }

    pub fn is_consumer(&self) -> bool {
        matches!(self, NodeKind::HostConsumer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepoRole {
    Pins,
    Lib,
    PkgsBase,
    Protocols,
    Producer,
    Integration,
    PkgsAggregator,
    Consumer,
    Root,
    External,
    Unknown,
}

impl RepoRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            RepoRole::Pins => "pins",
            RepoRole::Lib => "lib",
            RepoRole::PkgsBase => "pkgs-base",
            RepoRole::Protocols => "protocols",
            RepoRole::Producer => "producer",
            RepoRole::Integration => "integration",
            RepoRole::PkgsAggregator => "pkgs-aggregator",
            RepoRole::Consumer => "consumer",
            RepoRole::Root => "root",
            RepoRole::External => "external",
            RepoRole::Unknown => "unknown",
        }
    }

    pub fn layer(self) -> Option<u32> {
        Some(match self {
            RepoRole::Pins => 0,
            RepoRole::Lib | RepoRole::PkgsBase | RepoRole::Protocols => 1,
            RepoRole::Producer => 2,
            RepoRole::Integration => 3,
            RepoRole::PkgsAggregator => 4,
            RepoRole::Consumer => 5,
            RepoRole::Root => 6,
            RepoRole::External => 255,
            RepoRole::Unknown => return None,
        })
    }

    pub fn is_root(self) -> bool {
        matches!(self, RepoRole::Root)
    }

    pub fn is_producer(self) -> bool {
        self == RepoRole::Producer
    }

    pub fn is_consumer(self) -> bool {
        self == RepoRole::Consumer
    }

    pub fn is_pkgs_aggregator(self) -> bool {
        self == RepoRole::PkgsAggregator
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceNode {
    pub id: String,
    pub path: PathBuf,
    pub repo_url: Option<String>,
    pub kind: NodeKind,
    pub role: RepoRole,
    pub layer: Option<u32>,
    pub is_root: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceEdge {
    pub from: String,
    pub to: String,
    pub reason: EdgeReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EdgeReason {
    FlakeInput {
        input_name: String,
        lock_file: PathBuf,
    },
    Manual {
        source_file: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalInput {
    pub owner_node: String,
    pub input_name: String,
    pub locked_type: Option<String>,
    pub url_or_repo: Option<String>,
    pub rev: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceDag {
    pub nodes: BTreeMap<String, WorkspaceNode>,
    pub edges: Vec<WorkspaceEdge>,
    pub external_inputs: Vec<ExternalInput>,
    pub diagnostics: Vec<GraphDiagnostic>,
}

pub type NodeId = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakeNode {
    pub id: NodeId,
    pub name: String,
    pub path: PathBuf,
    pub remote: Option<String>,
    pub branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub input_name: String,
}

impl DependencyEdge {
    pub fn new(from: &str, to: &str, input_name: &str) -> Self {
        Self {
            from: from.to_string(),
            to: to.to_string(),
            input_name: input_name.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceGraph {
    pub root: NodeId,
    pub nodes: BTreeMap<NodeId, FlakeNode>,
    pub edges: Vec<DependencyEdge>,
}

impl WorkspaceGraph {
    pub fn get_node(&self, id: &NodeId) -> Option<&FlakeNode> {
        self.nodes.get(id)
    }

    pub fn dependents_of(&self, node_id: &NodeId) -> Vec<&DependencyEdge> {
        self.edges.iter().filter(|e| e.to == *node_id).collect()
    }

    pub fn dependencies_of(&self, node_id: &NodeId) -> Vec<&DependencyEdge> {
        self.edges.iter().filter(|e| e.from == *node_id).collect()
    }

    pub fn detect_cycles(&self) -> Result<(), Vec<NodeId>> {
        let mut visited: BTreeSet<&NodeId> = BTreeSet::new();
        let mut in_stack: BTreeSet<&NodeId> = BTreeSet::new();

        for node_id in self.nodes.keys() {
            if !visited.contains(node_id) {
                if let Some(cycle) = dfs_cycle(node_id, self, &mut visited, &mut in_stack) {
                    return Err(cycle);
                }
            }
        }

        Ok(())
    }

    pub fn topological_order(&self) -> Result<Vec<NodeId>, String> {
        self.detect_cycles().map_err(|cycle| {
            format!(
                "Cycle detected: {}",
                cycle
                    .iter()
                    .map(|n| n.as_str())
                    .collect::<Vec<_>>()
                    .join(" -> ")
            )
        })?;

        let mut in_degree: BTreeMap<&NodeId, usize> =
            self.nodes.keys().map(|id| (id, 0usize)).collect();
        for edge in &self.edges {
            *in_degree.entry(&edge.from).or_insert(0) += 1;
        }

        let mut queue: VecDeque<&NodeId> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(id, _)| *id)
            .collect();

        let mut result = Vec::new();
        while let Some(node_id) = queue.pop_front() {
            result.push(node_id.clone());
            for dep_edge in self.dependents_of(node_id) {
                if let Some(deg) = in_degree.get_mut(&dep_edge.from) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(&dep_edge.from);
                    }
                }
            }
        }

        if result.len() != self.nodes.len() {
            return Err("Graph contains a cycle (incomplete topological sort)".to_string());
        }

        Ok(result)
    }
}

fn dfs_cycle<'a>(
    node: &'a NodeId,
    graph: &'a WorkspaceGraph,
    visited: &mut BTreeSet<&'a NodeId>,
    in_stack: &mut BTreeSet<&'a NodeId>,
) -> Option<Vec<NodeId>> {
    visited.insert(node);
    in_stack.insert(node);

    for edge in &graph.edges {
        if edge.to == *node {
            let from = &edge.from;
            if !visited.contains(from) {
                if let Some(mut cycle) = dfs_cycle(from, graph, visited, in_stack) {
                    cycle.push(node.clone());
                    return Some(cycle);
                }
            } else if in_stack.contains(from) {
                return Some(vec![node.clone(), from.clone()]);
            }
        }
    }

    in_stack.remove(node);
    None
}

pub fn discover_graph(cfg: &WorkspaceConfig) -> Result<WorkspaceGraph, String> {
    let mut nodes = BTreeMap::new();
    let mut edges = Vec::new();

    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {}", e))?;

    for repo in &cfg.repos {
        let repo_path = repo.resolved_path(cfg);
        let repo_path = if repo_path.is_relative() {
            cwd.join(&repo_path)
        } else {
            repo_path
        };

        let branch = if repo_path.join(".git").exists() {
            git::git_branch(&repo_path).unwrap_or_else(|_| "main".to_string())
        } else {
            "main".to_string()
        };

        let remote = if repo_path.join(".git").exists() {
            git::git_remote(&repo_path, "origin").ok()
        } else {
            None
        };

        nodes.insert(
            repo.name.clone(),
            FlakeNode {
                id: repo.name.clone(),
                name: repo.name.clone(),
                path: repo_path,
                remote,
                branch,
            },
        );
    }

    for repo in &cfg.repos {
        let repo_path = repo.resolved_path(cfg);
        let deps: Vec<(String, String)> = scan_flake_inputs(&repo_path, cfg)?;

        for (dep_name, input_name) in &deps {
            if !nodes.contains_key(dep_name) {
                return Err(format!(
                    "Repo '{}' depends on '{}' which is not in the workspace config",
                    repo.name, dep_name
                ));
            }

            edges.push(DependencyEdge::new(&repo.name, dep_name, input_name));
        }
    }

    let root_id = if let Some(root) = cfg
        .repos
        .iter()
        .find(|r| r.name == "phenix" || r.name.contains("root"))
    {
        root.name.clone()
    } else {
        cfg.repos
            .first()
            .map(|r| r.name.clone())
            .unwrap_or_else(|| "root".to_string())
    };

    Ok(WorkspaceGraph {
        root: root_id,
        nodes,
        edges,
    })
}

fn scan_flake_inputs(
    repo_path: &Path,
    cfg: &WorkspaceConfig,
) -> Result<Vec<(String, String)>, String> {
    let flake_path = repo_path.join("flake.nix");
    if !flake_path.exists() {
        return Ok(Vec::new());
    }

    let content =
        std::fs::read_to_string(&flake_path).map_err(|e| format!("Read flake.nix: {}", e))?;
    let mut deps = Vec::new();

    for repo in &cfg.repos {
        if repo.name == "phenix" || repo_path == repo.resolved_path(cfg) {
            continue;
        }
        let check_patterns = [
            format!("./{}", repo.path),
            format!("\"{}\"", repo.name),
            format!("{}.url", repo.name),
        ];

        let matched = check_patterns
            .iter()
            .any(|pat| content.contains(pat.as_str()));

        if matched {
            deps.push((repo.name.clone(), repo.name.clone()));
        }
    }

    Ok(deps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_graph(edges: Vec<(&str, &str)>) -> WorkspaceGraph {
        let mut nodes = BTreeMap::new();
        let mut all_names = BTreeSet::new();
        for (from, to) in &edges {
            all_names.insert(from.to_string());
            all_names.insert(to.to_string());
        }
        for name in &all_names {
            let dir = std::env::temp_dir().join(format!("__test_{}", name));
            nodes.insert(
                name.clone(),
                FlakeNode {
                    id: name.clone(),
                    name: name.clone(),
                    path: dir,
                    remote: None,
                    branch: "main".to_string(),
                },
            );
        }
        let edges: Vec<DependencyEdge> = edges
            .into_iter()
            .map(|(from, to)| DependencyEdge::new(from, to, to))
            .collect();
        let root = all_names.iter().next().cloned().unwrap_or_default();
        WorkspaceGraph { root, nodes, edges }
    }

    #[test]
    fn test_topo_order_simple() {
        let graph = make_graph(vec![
            ("root", "shell"),
            ("root", "tools"),
            ("shell", "tools"),
        ]);
        let order = graph.topological_order().unwrap();
        assert_eq!(order, vec!["tools", "shell", "root"]);
    }

    #[test]
    fn test_topo_order_linear() {
        let graph = make_graph(vec![("c", "b"), ("b", "a")]);
        let order = graph.topological_order().unwrap();
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_cycle_detection() {
        let graph = make_graph(vec![("a", "b"), ("b", "a")]);
        assert!(graph.topological_order().is_err());
    }

    #[test]
    fn test_cycle_detection_longer() {
        let graph = make_graph(vec![("a", "b"), ("b", "c"), ("c", "a")]);
        assert!(graph.topological_order().is_err());
    }

    #[test]
    fn test_no_deps() {
        let graph = make_graph(vec![]);
        let order = graph.topological_order().unwrap();
        assert_eq!(order.len(), 0);
    }

    #[test]
    fn test_single_node() {
        let graph = make_graph(vec![("a", "a")]);
        assert!(graph.topological_order().is_err());
    }

    #[test]
    fn test_dependents_of() {
        let graph = make_graph(vec![("root", "a"), ("root", "b"), ("shell", "a")]);
        let deps = graph.dependents_of(&"a".to_string());
        assert_eq!(deps.len(), 2);
        assert!(deps.iter().any(|e| e.from == "root"));
        assert!(deps.iter().any(|e| e.from == "shell"));
    }

    #[test]
    fn test_dependencies_of() {
        let graph = make_graph(vec![("root", "a"), ("root", "b")]);
        let deps = graph.dependencies_of(&"root".to_string());
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn test_push_order_equals_topo_order() {
        let graph = make_graph(vec![
            ("root", "shell"),
            ("root", "tools"),
            ("shell", "tools"),
        ]);
        let order = graph.topological_order().unwrap();
        assert_eq!(order, vec!["tools", "shell", "root"]);
    }
}
