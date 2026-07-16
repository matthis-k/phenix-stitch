use std::collections::{BTreeMap, HashSet};
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
    FlakeInput {
        input_name: String,
        lock_file: PathBuf,
    },
    Manual {
        source_file: PathBuf,
    },
    SubmoduleMembership {
        path: PathBuf,
        gitlink: Option<String>,
    },
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
        matches!(
            self.kind,
            EdgeKind::FlakeInput { .. } | EdgeKind::Manual { .. }
        )
    }

    pub fn dedup_key(&self) -> (String, String, Option<String>, &'static str) {
        let kind = match self.kind {
            EdgeKind::FlakeInput { .. } => "flake-input",
            EdgeKind::Manual { .. } => "manual",
            EdgeKind::SubmoduleMembership { .. } => "submodule-membership",
        };
        (
            self.from.clone(),
            self.to.clone(),
            self.input_name().map(str::to_owned),
            kind,
        )
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
        Self {
            nodes,
            edges: Vec::new(),
            external_inputs: Vec::new(),
            diagnostics: Vec::new(),
        }
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
        self.edges
            .iter()
            .filter(|edge| edge.is_semantic_dependency())
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
        Self {
            strategy,
            message: message.into(),
        }
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
