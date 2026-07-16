#!/usr/bin/env python3
"""Remove retired Stitch graph generations and legacy adapters."""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def rewrite(path: str, transform) -> None:
    target = ROOT / path
    text = target.read_text()
    updated = transform(text)
    if updated != text:
        target.write_text(updated)


def set_content(path: str, content: str) -> None:
    target = ROOT / path
    if target.read_text() != content:
        target.write_text(content)


# Rename the one remaining execution-only graph so it is not confused with the
# canonical dependency graph. Retired DAG/node/edge names disappear entirely.
renames = {
    r"\bWorkspaceGraph\b": "SyncGraph",
    r"\bFlakeNode\b": "SyncNode",
    r"\bDependencyEdge\b": "SyncEdge",
    r"\bWorkspaceDag\b": "WorkspaceGraphDraft",
    r"\bWorkspaceNode\b": "NodeSpec",
    r"\bWorkspaceEdge\b": "EdgeSpec",
    r"\bEdgeReason\b": "EdgeKind",
    r"\bfrom_legacy\b": "from_snapshot",
    r"\bto_legacy_dag\b": "to_snapshot",
    r"\bto_legacy_workspace_graph\b": "to_sync_graph",
}
for path in (ROOT / "crates/stitch/src").rglob("*.rs"):
    def transform(source: str) -> str:
        for pattern, replacement in renames.items():
            source = re.sub(pattern, replacement, source)
        return source
    rewrite(str(path.relative_to(ROOT)), transform)

set_content(
    "crates/stitch/src/graph/mod.rs",
    r'''use std::collections::{BTreeMap, BTreeSet, VecDeque};
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
''',
)

set_content(
    "crates/stitch/src/graph/spec.rs",
    r'''use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::graph::validate::GraphDiagnostic;
use crate::graph::{ExternalInput, NodeKind, RepoRole};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeSpec {
    pub id: String,
    pub path: PathBuf,
    pub repo_url: Option<String>,
    pub kind: NodeKind,
    pub role: RepoRole,
    pub layer: Option<u32>,
    pub is_root: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    FlakeInput { input_name: String, lock_file: PathBuf },
    Manual { source_file: PathBuf },
    SubmoduleMembership { path: PathBuf, gitlink: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EdgeSpec {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
}

impl EdgeSpec {
    pub fn input_name(&self) -> Option<&str> {
        match &self.kind {
            EdgeKind::FlakeInput { input_name, .. } => Some(input_name),
            _ => None,
        }
    }

    pub fn is_semantic_dependency(&self) -> bool {
        matches!(self.kind, EdgeKind::FlakeInput { .. } | EdgeKind::Manual { .. })
    }

    pub fn dedup_key(&self) -> (String, String, Option<String>, &'static str) {
        let kind = match self.kind {
            EdgeKind::FlakeInput { .. } => "flake-input",
            EdgeKind::Manual { .. } => "manual",
            EdgeKind::SubmoduleMembership { .. } => "submodule-membership",
        };
        (self.from.clone(), self.to.clone(), self.input_name().map(str::to_owned), kind)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceGraphDraft {
    pub nodes: BTreeMap<String, NodeSpec>,
    pub edges: Vec<EdgeSpec>,
    pub external_inputs: Vec<ExternalInput>,
    pub diagnostics: Vec<GraphDiagnostic>,
}

impl WorkspaceGraphDraft {
    pub fn new(nodes: BTreeMap<String, NodeSpec>) -> Self {
        Self { nodes, edges: Vec::new(), external_inputs: Vec::new(), diagnostics: Vec::new() }
    }

    pub fn merge(&mut self, other: Self) {
        self.nodes.extend(other.nodes);
        self.edges.extend(other.edges);
        self.external_inputs.extend(other.external_inputs);
        self.diagnostics.extend(other.diagnostics);
        self.dedup_edges();
    }

    pub fn dedup_edges(&mut self) {
        let mut seen = HashSet::new();
        self.edges.retain(|edge| seen.insert(edge.dedup_key()));
    }

    pub fn semantic_edges(&self) -> impl Iterator<Item = &EdgeSpec> {
        self.edges.iter().filter(|edge| edge.is_semantic_dependency())
    }
}

#[derive(Debug, Clone)]
pub struct GenerationContext {
    pub root: PathBuf,
    pub metadata: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct StrategyError {
    pub strategy: &'static str,
    pub message: String,
}

impl StrategyError {
    pub fn new(strategy: &'static str, message: impl Into<String>) -> Self {
        Self { strategy, message: message.into() }
    }
}

impl std::fmt::Display for StrategyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.strategy, self.message)
    }
}

impl std::error::Error for StrategyError {}

pub trait DagGenerationStrategy {
    fn name(&self) -> &'static str;
    fn generate(&self, ctx: &GenerationContext) -> Result<WorkspaceGraphDraft, StrategyError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_dedups_by_input_name_not_repo_only() {
        let mut draft = WorkspaceGraphDraft::default();
        for _ in 0..2 {
            draft.edges.push(EdgeSpec {
                from: "consumer".into(),
                to: "provider".into(),
                kind: EdgeKind::FlakeInput {
                    input_name: "provider-pin".into(),
                    lock_file: "flake.lock".into(),
                },
            });
        }
        draft.dedup_edges();
        assert_eq!(draft.edges.len(), 1);
    }
}
''',
)

set_content(
    "crates/stitch/src/graph/derive.rs",
    r'''use std::path::Path;

use crate::graph::spec::{DagGenerationStrategy, GenerationContext};
use crate::graph::strategy::FlakeLocksStrategy;
use crate::graph::CanonicalWorkspaceGraph;
use crate::model::WorkspaceConfig;

#[derive(Debug)]
pub enum GraphError {
    Io(String),
    Parse(String),
    Validation(String),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "I/O error: {msg}"),
            Self::Parse(msg) => write!(f, "parse error: {msg}"),
            Self::Validation(msg) => write!(f, "validation error: {msg}"),
        }
    }
}

impl std::error::Error for GraphError {}

pub fn derive_workspace_graph(
    root: &Path,
    metadata: Option<&Path>,
) -> Result<CanonicalWorkspaceGraph, GraphError> {
    let context = GenerationContext {
        root: root.to_path_buf(),
        metadata: metadata.map(Path::to_path_buf),
    };
    let draft = FlakeLocksStrategy
        .generate(&context)
        .map_err(|error| GraphError::Parse(error.to_string()))?;
    CanonicalWorkspaceGraph::from_draft(draft)
        .map_err(|error| GraphError::Validation(error.to_string()))
}

pub fn derive_workspace_graph_from_config(
    config: &WorkspaceConfig,
    metadata: Option<&Path>,
) -> Result<CanonicalWorkspaceGraph, GraphError> {
    let draft = FlakeLocksStrategy
        .generate_from_config(config, metadata)
        .map_err(|error| GraphError::Parse(error.to_string()))?;
    CanonicalWorkspaceGraph::from_draft(draft)
        .map_err(|error| GraphError::Validation(error.to_string()))
}
''',
)

set_content(
    "crates/stitch/src/graph/topo.rs",
    r'''use crate::graph::planner::{
    DagPlanRequest, DagPlanner, PlanClosureMode, PlanOrderMode, PlanSelectionMode,
};
use crate::graph::CanonicalWorkspaceGraph;

#[derive(Debug)]
pub struct TopoError {
    pub cycle_nodes: Vec<String>,
}

impl std::fmt::Display for TopoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cycle detected: {}", self.cycle_nodes.join(" -> "))
    }
}

impl std::error::Error for TopoError {}

pub fn provider_before_consumer_order(
    graph: &CanonicalWorkspaceGraph,
) -> Result<Vec<String>, TopoError> {
    let stable_order = graph.node_ids().cloned().collect();
    let plan = DagPlanner::new(graph)
        .plan(&DagPlanRequest {
            selection: PlanSelectionMode::All,
            explicit_nodes: Vec::new(),
            closure: PlanClosureMode::All,
            order: PlanOrderMode::ProvidersFirst,
            stable_order,
        })
        .map_err(|error| TopoError { cycle_nodes: vec![error] })?;
    Ok(plan.nodes.into_iter().map(|node| node.name).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeKind, EdgeSpec, NodeKind, NodeSpec, RepoRole, WorkspaceGraphDraft};

    fn graph(edges: Vec<(&str, &str)>) -> CanonicalWorkspaceGraph {
        let mut draft = WorkspaceGraphDraft::default();
        for id in edges.iter().flat_map(|(from, to)| [*from, *to]) {
            draft.nodes.entry(id.to_string()).or_insert(NodeSpec {
                id: id.to_string(),
                path: id.into(),
                repo_url: None,
                kind: NodeKind::Unknown,
                role: RepoRole::Unknown,
                layer: None,
                is_root: false,
            });
        }
        draft.edges = edges
            .into_iter()
            .map(|(from, to)| EdgeSpec {
                from: from.to_string(),
                to: to.to_string(),
                kind: EdgeKind::Manual { source_file: "test".into() },
            })
            .collect();
        CanonicalWorkspaceGraph::from_draft(draft).unwrap()
    }

    #[test]
    fn providers_precede_consumers() {
        let graph = graph(vec![("packages", "pins"), ("hosts", "packages")]);
        assert_eq!(
            provider_before_consumer_order(&graph).unwrap(),
            vec!["pins", "packages", "hosts"]
        );
    }

    #[test]
    fn cycles_are_rejected() {
        assert!(provider_before_consumer_order(&graph(vec![("a", "b"), ("b", "a")])).is_err());
    }
}
''',
)

# Canonical graph conversions now use the current draft shape only.
def canonical(source: str) -> str:
    source = source.replace("use crate::graph::{WorkspaceGraphDraft, EdgeSpec};\n", "")
    source = re.sub(
        r"\n    pub fn from_snapshot\(graph: WorkspaceGraphDraft\) -> Result<Self, CanonicalizeError> \{.*?\n    \}\n",
        "\n",
        source,
        flags=re.DOTALL,
    )
    source = re.sub(
        r"    pub fn to_snapshot\(&self\) -> WorkspaceGraphDraft \{.*?\n    \}\n\n    pub fn to_sync_graph",
        '''    pub(crate) fn to_snapshot(&self) -> WorkspaceGraphDraft {
        let nodes = self
            .id_to_index
            .iter()
            .filter_map(|(id, index)| {
                self.graph
                    .node_weight(*index)
                    .cloned()
                    .map(|node| (id.clone(), node))
            })
            .collect();
        let edges = self.graph.edge_weights().cloned().collect();
        WorkspaceGraphDraft {
            nodes,
            edges,
            external_inputs: self.external_inputs.clone(),
            diagnostics: self.diagnostics.clone(),
        }
    }

    pub(crate) fn to_sync_graph''',
        source,
        flags=re.DOTALL,
    )
    source = source.replace("crate::graph::SyncNode", "crate::graph::SyncNode")
    source = source.replace("crate::graph::SyncEdge::new", "crate::graph::SyncEdge::new")
    return source
rewrite("crates/stitch/src/graph/canonical.rs", canonical)

# Validation keeps a private serializable draft implementation while exposing
# only the canonical graph at the public boundary.
def validation(source: str) -> str:
    source = source.replace("validate_graph(", "validate_snapshot(")
    source = source.replace(
        "pub fn validate_canonical_graph(\n    graph: &crate::graph::CanonicalWorkspaceGraph,",
        "pub fn validate_graph(\n    graph: &crate::graph::CanonicalWorkspaceGraph,",
    )
    source = source.replace("validate_snapshot(&graph.to_snapshot(), opts)", "validate_snapshot(&graph.to_snapshot(), opts)")
    source = source.replace("pub fn validate_snapshot(graph:", "fn validate_snapshot(graph:")
    source = source.replace("validate_canonical_graph_preserves_rule_coverage", "validate_graph_preserves_rule_coverage")
    return source
rewrite("crates/stitch/src/graph/validate.rs", validation)

# Rendering uses a private draft snapshot and canonical public functions.
def rendering(source: str) -> str:
    source = source.replace(
        "use crate::graph::{validate::GraphValidationReport, WorkspaceGraphDraft};",
        "use crate::graph::{validate::GraphValidationReport, CanonicalWorkspaceGraph, WorkspaceGraphDraft};",
    )
    source = source.replace("pub fn render_graph_derive(", "fn render_graph_snapshot(", 1)
    source = source.replace("pub fn render_order(\n", "fn render_order_snapshot(\n", 1)
    insert = '''
pub fn render_graph_derive(
    graph: &CanonicalWorkspaceGraph,
    format: RenderFormat,
) -> Result<String, String> {
    render_graph_snapshot(&graph.to_snapshot(), format)
}

pub fn render_order(
    graph: &CanonicalWorkspaceGraph,
    order: &[String],
    format: RenderFormat,
) -> Result<String, String> {
    render_order_snapshot(&graph.to_snapshot(), order, format)
}

'''
    marker = "pub fn render_validation_report("
    source = source.replace(marker, insert + marker, 1)
    source = re.sub(
        r"\s*let canonical =\s*crate::graph::CanonicalWorkspaceGraph::from_snapshot\(graph\).*?;\n",
        "\n        let canonical = graph;\n",
        source,
        flags=re.DOTALL,
    )
    return source
rewrite("crates/stitch/src/graph/render.rs", rendering)

# Execution planning receives the canonical graph directly.
def execution(source: str) -> str:
    source = source.replace("for edge in &dag.edges {", "for edge in dag.semantic_edges() {")
    source = source.replace(
        "graph::CanonicalWorkspaceGraph::from_snapshot(dag).map_err(|e| e.to_string())",
        "Ok(dag)",
    )
    return source
rewrite("crates/stitch/src/exec.rs", execution)

# Remove obsolete public re-exports and point sync callers at the explicit graph.
def library(source: str) -> str:
    source = source.replace("validate_canonical_graph, ", "")
    source = re.sub(
        r"\n    EdgeKind, ExternalInput, GraphSource, NodeKind, RepoRole, WorkspaceGraphDraft, EdgeSpec,\n    NodeSpec,\n",
        "\n    EdgeKind, EdgeSpec, ExternalInput, NodeKind, NodeSpec, RepoRole, WorkspaceGraphDraft,\n",
        source,
    )
    return source
rewrite("crates/stitch/src/lib.rs", library)

for path in [
    "crates/stitch-cli/src/main.rs",
    "crates/stitch-mcp/src/tools.rs",
]:
    rewrite(path, lambda source: source.replace("graph::discover_graph", "graph::discover_sync_graph"))

# No retired names or compatibility markers may remain in production code.
for path in (ROOT / "crates").rglob("*.rs"):
    text = path.read_text()
    if re.search(r"\bWorkspaceDag\b|\bWorkspaceNode\b|\bWorkspaceEdge\b|\bFlakeNode\b|\bDependencyEdge\b|from_legacy|to_legacy", text):
        raise SystemExit(f"retired graph API remains in {path.relative_to(ROOT)}")
