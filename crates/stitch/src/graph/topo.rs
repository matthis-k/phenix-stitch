use crate::graph::planner::{
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
        .map_err(|error| TopoError {
            cycle_nodes: vec![error],
        })?;
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
                kind: EdgeKind::Manual {
                    source_file: "test".into(),
                },
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
