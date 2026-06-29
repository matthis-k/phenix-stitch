use std::collections::BTreeMap;
use std::path::Path;

use crate::graph::inventory::discover_inventory;
use crate::graph::lock::{
    build_workspace_aliases, external_from_lock, input_target_name, map_lock_target_to_workspace,
    parse_flake_lock,
};
use crate::graph::validate::GraphDiagnostic;
use crate::graph::{EdgeReason, WorkspaceDag, WorkspaceEdge, WorkspaceNode};

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
    let discovery = discover_inventory(root, metadata).map_err(GraphError::Io)?;
    let aliases = build_workspace_aliases(&discovery.nodes);

    let mut graph = WorkspaceDag::new(discovery.nodes);

    let node_ids: Vec<String> = graph.nodes.keys().cloned().collect();

    for node_id in &node_ids {
        let node = graph.nodes.get(node_id).unwrap();
        let lock_path = node.path.join("flake.lock");

        if !lock_path.exists() {
            graph.diagnostics.push(GraphDiagnostic::warn(
                "missing_flake_lock",
                format!(
                    "node '{node_id}' has no flake.lock at {}",
                    lock_path.display()
                ),
                vec![node_id.clone()],
            ));
            continue;
        }

        let lock = match parse_flake_lock(&lock_path) {
            Ok(l) => l,
            Err(e) => {
                graph.diagnostics.push(GraphDiagnostic::error(
                    "parse_flake_lock_failed",
                    format!("node '{node_id}': {e}"),
                    vec![node_id.clone()],
                ));
                continue;
            }
        };

        let root_lock_node_name = lock.root.as_deref().unwrap_or("root");
        let Some(root_lock_node) = lock.nodes.get(root_lock_node_name) else {
            graph.diagnostics.push(GraphDiagnostic::error(
                "lock_root_node_missing",
                format!("node '{node_id}': lock root node '{root_lock_node_name}' not found"),
                vec![node_id.clone()],
            ));
            continue;
        };

        let Some(inputs) = &root_lock_node.inputs else {
            continue;
        };

        for (input_name, input_value) in inputs {
            let Some(lock_target_name) = input_target_name(input_value) else {
                continue;
            };

            let Some(target_lock_node) = lock.nodes.get(&lock_target_name) else {
                graph.diagnostics.push(GraphDiagnostic::error(
                    "input_target_missing",
                    format!(
                        "node '{node_id}': input '{input_name}' targets '{lock_target_name}' not found in lock"
                    ),
                    vec![node_id.clone()],
                ));
                continue;
            };

            if let Some(workspace_target_id) =
                map_lock_target_to_workspace(&lock_target_name, target_lock_node, &aliases)
            {
                if workspace_target_id != *node_id {
                    graph.edges.push(WorkspaceEdge {
                        from: node_id.clone(),
                        to: workspace_target_id,
                        reason: EdgeReason::FlakeInput {
                            input_name: input_name.clone(),
                            lock_file: lock_path.clone(),
                        },
                    });
                }
            } else {
                graph.external_inputs.push(external_from_lock(
                    node_id.clone(),
                    input_name.clone(),
                    target_lock_node,
                ));
            }
        }
    }

    graph.dedup_edges();
    Ok(graph)
}
