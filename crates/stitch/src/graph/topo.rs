use crate::graph::canonical::CanonicalWorkspaceGraph;
use crate::graph::planner::{
    DagPlanRequest, DagPlanner, PlanClosureMode, PlanOrderMode, PlanSelectionMode,
};
use crate::graph::WorkspaceDag;

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

/// Compute provider-before-consumer topological order.
///
/// Given edges consumer -> provider (where consumer depends on provider),
/// providers must come before consumers.
///
/// Algorithm:
/// 1. Build reverse adjacency: provider -> consumers
/// 2. All nodes with indegree 0 (no dependencies) are ready.
/// 3. Process in ascending layer, then ascending id order.
pub fn provider_before_consumer_order(graph: &WorkspaceDag) -> Result<Vec<String>, TopoError> {
    let canonical = CanonicalWorkspaceGraph::from_legacy(graph.clone()).map_err(|e| TopoError {
        cycle_nodes: vec![e.to_string()],
    })?;
    let stable_order = graph.nodes.keys().cloned().collect();
    let plan = DagPlanner::new(&canonical)
        .plan(&DagPlanRequest {
            selection: PlanSelectionMode::All,
            explicit_nodes: Vec::new(),
            closure: PlanClosureMode::All,
            order: PlanOrderMode::ProvidersFirst,
            stable_order,
        })
        .map_err(|e| TopoError {
            cycle_nodes: vec![e],
        })?;
    Ok(plan.nodes.into_iter().map(|node| node.name).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeReason, NodeKind, RepoRole, WorkspaceEdge, WorkspaceNode};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn make_node(id: &str, layer: Option<u32>) -> WorkspaceNode {
        WorkspaceNode {
            id: id.to_string(),
            path: PathBuf::new(),
            repo_url: None,
            kind: NodeKind::Unknown,
            role: RepoRole::Unknown,
            layer,
            is_root: false,
        }
    }

    fn make_edge(from: &str, to: &str) -> WorkspaceEdge {
        WorkspaceEdge {
            from: from.to_string(),
            to: to.to_string(),
            reason: EdgeReason::Manual {
                source_file: PathBuf::from("test"),
            },
        }
    }

    fn make_dag(nodes: Vec<WorkspaceNode>, edges: Vec<WorkspaceEdge>) -> WorkspaceDag {
        let mut node_map = BTreeMap::new();
        for n in nodes {
            node_map.insert(n.id.clone(), n);
        }
        WorkspaceDag {
            nodes: node_map,
            edges,
            external_inputs: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn test_provider_before_consumer_basic() {
        // edges: packages -> pins, hosts -> packages, root -> hosts
        // Expected: pins, packages, hosts, root
        let nodes = vec![
            make_node("pins", Some(0)),
            make_node("packages", Some(1)),
            make_node("hosts", Some(3)),
            make_node("root", Some(4)),
        ];
        let edges = vec![
            make_edge("packages", "pins"),
            make_edge("hosts", "packages"),
            make_edge("root", "hosts"),
        ];
        let dag = make_dag(nodes, edges);
        let order = provider_before_consumer_order(&dag).unwrap();
        assert_eq!(order, vec!["pins", "packages", "hosts", "root"]);
    }

    #[test]
    fn test_provider_before_consumer_cycle() {
        let nodes = vec![make_node("a", None), make_node("b", None)];
        let edges = vec![make_edge("a", "b"), make_edge("b", "a")];
        let dag = make_dag(nodes, edges);
        assert!(provider_before_consumer_order(&dag).is_err());
    }

    #[test]
    fn test_order_deterministic_by_layer_and_id() {
        let nodes = vec![
            make_node("z-pins", Some(0)),
            make_node("a-pins", Some(0)),
            make_node("packages", Some(1)),
        ];
        let edges = vec![
            make_edge("packages", "z-pins"),
            make_edge("packages", "a-pins"),
        ];
        let dag = make_dag(nodes, edges);
        let order = provider_before_consumer_order(&dag).unwrap();
        // Same layer, sorted by id: a-pins before z-pins
        assert_eq!(order[0], "a-pins");
        assert_eq!(order[1], "z-pins");
        assert_eq!(order[2], "packages");
    }
}
