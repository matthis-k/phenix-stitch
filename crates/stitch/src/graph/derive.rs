use std::path::Path;

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
