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
    validate_graph, DiagnosticSeverity, GraphDiagnostic, GraphValidationReport, ValidateOptions,
};

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
            Self::Pins
                | Self::PackageProvider
                | Self::ToolProvider
                | Self::ShellProvider
                | Self::DesktopProvider
        )
    }

    pub fn is_consumer(&self) -> bool {
        matches!(self, Self::HostConsumer)
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
            Self::Pins => "pins",
            Self::Lib => "lib",
            Self::PkgsBase => "pkgs-base",
            Self::Protocols => "protocols",
            Self::Producer => "producer",
            Self::Integration => "integration",
            Self::PkgsAggregator => "pkgs-aggregator",
            Self::Consumer => "consumer",
            Self::Root => "root",
            Self::External => "external",
            Self::Unknown => "unknown",
        }
    }

    pub fn layer(self) -> Option<u32> {
        Some(match self {
            Self::Pins => 0,
            Self::Lib | Self::PkgsBase | Self::Protocols => 1,
            Self::Producer => 2,
            Self::Integration => 3,
            Self::PkgsAggregator => 4,
            Self::Consumer => 5,
            Self::Root => 6,
            Self::External => 255,
            Self::Unknown => return None,
        })
    }

    pub fn is_root(self) -> bool {
        matches!(self, Self::Root)
    }

    pub fn is_producer(self) -> bool {
        self == Self::Producer
    }

    pub fn is_consumer(self) -> bool {
        self == Self::Consumer
    }

    pub fn is_pkgs_aggregator(self) -> bool {
        self == Self::PkgsAggregator
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalInput {
    pub owner_node: String,
    pub input_name: String,
    pub locked_type: Option<String>,
    pub url_or_repo: Option<String>,
    pub rev: Option<String>,
}

pub type NodeId = String;

/// Execution-oriented repository graph used by transactional synchronization.
/// This is distinct from the canonical dependency graph because it snapshots
/// branch and remote state needed for mutation safety.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncNode {
    pub id: NodeId,
    pub name: String,
    pub path: PathBuf,
    pub remote: Option<String>,
    pub branch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncEdge {
    pub from: NodeId,
    pub to: NodeId,
    pub input_name: String,
}

impl SyncEdge {
    pub fn new(from: &str, to: &str, input_name: &str) -> Self {
        Self {
            from: from.to_string(),
            to: to.to_string(),
            input_name: input_name.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncGraph {
    pub root: NodeId,
    pub nodes: BTreeMap<NodeId, SyncNode>,
    pub edges: Vec<SyncEdge>,
}

impl SyncGraph {
    pub fn get_node(&self, id: &NodeId) -> Option<&SyncNode> {
        self.nodes.get(id)
    }

    pub fn dependents_of(&self, node_id: &NodeId) -> Vec<&SyncEdge> {
        self.edges.iter().filter(|edge| edge.to == *node_id).collect()
    }

    pub fn dependencies_of(&self, node_id: &NodeId) -> Vec<&SyncEdge> {
        self.edges.iter().filter(|edge| edge.from == *node_id).collect()
    }

    fn detect_cycles(&self) -> Result<(), Vec<NodeId>> {
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
                cycle.iter().map(String::as_str).collect::<Vec<_>>().join(" -> ")
            )
        })?;

        let mut in_degree: BTreeMap<&NodeId, usize> =
            self.nodes.keys().map(|id| (id, 0usize)).collect();
        for edge in &self.edges {
            *in_degree.entry(&edge.from).or_insert(0) += 1;
        }
        let mut queue: VecDeque<&NodeId> = in_degree
            .iter()
            .filter(|(_, degree)| **degree == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut result = Vec::new();
        while let Some(node_id) = queue.pop_front() {
            result.push(node_id.clone());
            for edge in self.dependents_of(node_id) {
                if let Some(degree) = in_degree.get_mut(&edge.from) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(&edge.from);
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
    graph: &'a SyncGraph,
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

pub fn discover_sync_graph(cfg: &WorkspaceConfig) -> Result<SyncGraph, String> {
    let mut nodes = BTreeMap::new();
    let mut edges = Vec::new();
    let cwd = std::env::current_dir().map_err(|e| format!("Cannot get cwd: {e}"))?;

    for repo in &cfg.repos {
        let repo_path = repo.resolved_path(cfg);
        let repo_path = if repo_path.is_relative() { cwd.join(repo_path) } else { repo_path };
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
            SyncNode {
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
        for (dependency, input_name) in scan_flake_inputs(&repo_path, cfg)? {
            if !nodes.contains_key(&dependency) {
                return Err(format!(
                    "Repo '{}' depends on '{}' which is not in the workspace config",
                    repo.name, dependency
                ));
            }
            edges.push(SyncEdge::new(&repo.name, &dependency, &input_name));
        }
    }

    let root = cfg
        .repos
        .iter()
        .find(|repo| repo.name == cfg.workspace || repo.name.contains("root"))
        .or_else(|| cfg.repos.first())
        .map(|repo| repo.name.clone())
        .unwrap_or_else(|| "root".to_string());
    Ok(SyncGraph { root, nodes, edges })
}

fn scan_flake_inputs(
    repo_path: &Path,
    cfg: &WorkspaceConfig,
) -> Result<Vec<(String, String)>, String> {
    let flake_path = repo_path.join("flake.nix");
    if !flake_path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&flake_path)
        .map_err(|e| format!("Read flake.nix: {e}"))?;
    let mut dependencies = Vec::new();
    for repo in &cfg.repos {
        if repo.name == cfg.workspace || repo_path == repo.resolved_path(cfg) {
            continue;
        }
        let patterns = [
            format!("./{}", repo.path),
            format!("\"{}\"", repo.name),
            format!("{}.url", repo.name),
        ];
        if patterns.iter().any(|pattern| content.contains(pattern)) {
            dependencies.push((repo.name.clone(), repo.name.clone()));
        }
    }
    Ok(dependencies)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_graph(edges: Vec<(&str, &str)>) -> SyncGraph {
        let mut names = BTreeSet::new();
        for (from, to) in &edges {
            names.insert(from.to_string());
            names.insert(to.to_string());
        }
        let nodes = names
            .iter()
            .map(|name| {
                (
                    name.clone(),
                    SyncNode {
                        id: name.clone(),
                        name: name.clone(),
                        path: PathBuf::from(name),
                        remote: None,
                        branch: "main".to_string(),
                    },
                )
            })
            .collect();
        let edges = edges
            .into_iter()
            .map(|(from, to)| SyncEdge::new(from, to, to))
            .collect();
        let root = names.iter().next().cloned().unwrap_or_default();
        SyncGraph { root, nodes, edges }
    }

    #[test]
    fn sync_graph_orders_dependencies_first() {
        let graph = make_graph(vec![("root", "shell"), ("shell", "tools")]);
        assert_eq!(graph.topological_order().unwrap(), vec!["tools", "shell", "root"]);
    }

    #[test]
    fn sync_graph_rejects_cycles() {
        assert!(make_graph(vec![("a", "b"), ("b", "a")])
            .topological_order()
            .is_err());
    }
}
