use std::collections::BTreeMap;
use std::path::Path;

use crate::graph::spec::{DagGenerationStrategy, GenerationContext};
use crate::graph::strategy::FlakeLocksStrategy;
use crate::graph::{WorkspaceDag, WorkspaceNode};

#[derive(Debug)]
pub enum GraphError {
    Io(String),
    Parse(String),
    Validation(String),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::Io(msg) => write!(f, "I/O error: {msg}"),
            GraphError::Parse(msg) => write!(f, "parse error: {msg}"),
            GraphError::Validation(msg) => write!(f, "validation error: {msg}"),
        }
    }
}

impl std::error::Error for GraphError {}

impl WorkspaceDag {
    pub fn new(nodes: BTreeMap<String, WorkspaceNode>) -> Self {
        WorkspaceDag {
            nodes,
            edges: Vec::new(),
            external_inputs: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    pub fn dedup_edges(&mut self) {
        let mut unique = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for edge in self.edges.drain(..) {
            let key = (edge.from.clone(), edge.to.clone());
            if seen.insert(key) {
                unique.push(edge);
            }
        }
        self.edges = unique;
    }
}

pub fn derive_graph_from_locks(
    root: &Path,
    metadata: Option<&Path>,
) -> Result<WorkspaceDag, GraphError> {
    let ctx = GenerationContext {
        root: root.to_path_buf(),
        metadata: metadata.map(|p| p.to_path_buf()),
    };
    let draft = FlakeLocksStrategy
        .generate(&ctx)
        .map_err(|e| GraphError::Parse(e.to_string()))?;
    Ok(draft.into())
}
