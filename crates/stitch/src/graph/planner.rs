use std::collections::{BTreeMap, BTreeSet};

use crate::graph::canonical::CanonicalWorkspaceGraph;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanSelectionMode {
    All,
    Explicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanClosureMode {
    SelfOnly,
    Upstream,
    Downstream,
    Connected,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanOrderMode {
    Stable,
    ProvidersFirst,
    ConsumersFirst,
}

#[derive(Debug, Clone)]
pub struct DagPlanRequest {
    pub selection: PlanSelectionMode,
    pub explicit_nodes: Vec<String>,
    pub closure: PlanClosureMode,
    pub order: PlanOrderMode,
    pub stable_order: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PlannedDagNode {
    pub name: String,
    pub directly_selected: bool,
    pub downstream_only: bool,
}

#[derive(Debug, Clone)]
pub struct DagPlan {
    pub nodes: Vec<PlannedDagNode>,
}

#[derive(Debug, Clone)]
pub struct DagPlanner<'a> {
    graph: &'a CanonicalWorkspaceGraph,
}

impl<'a> DagPlanner<'a> {
    pub fn new(graph: &'a CanonicalWorkspaceGraph) -> Self {
        Self { graph }
    }

    pub fn plan(&self, request: &DagPlanRequest) -> Result<DagPlan, String> {
        let all_nodes: Vec<String> = if request.stable_order.is_empty() {
            self.graph.node_ids().cloned().collect()
        } else {
            request.stable_order.clone()
        };
        let selected = match request.selection {
            PlanSelectionMode::All => all_nodes.clone(),
            PlanSelectionMode::Explicit => {
                for node in &request.explicit_nodes {
                    if self.graph.node(node).is_none() {
                        return Err(format!("Unknown node: {node}"));
                    }
                }
                request.explicit_nodes.clone()
            }
        };
        let closure_nodes = self.expand_closure(&selected, request.closure, &all_nodes);
        let ordered = self.order_nodes(&closure_nodes, request.order, &all_nodes)?;
        let selected_set: BTreeSet<&String> = selected.iter().collect();
        let downstream_set: BTreeSet<String> = self
            .expand_closure(&selected, PlanClosureMode::Downstream, &all_nodes)
            .into_iter()
            .collect();
        Ok(DagPlan {
            nodes: ordered
                .into_iter()
                .map(|name| PlannedDagNode {
                    directly_selected: selected_set.contains(&name),
                    downstream_only: downstream_set.contains(&name)
                        && !selected_set.contains(&name),
                    name,
                })
                .collect(),
        })
    }

    pub fn expand_closure(
        &self,
        selected: &[String],
        closure: PlanClosureMode,
        all_nodes: &[String],
    ) -> Vec<String> {
        match closure {
            PlanClosureMode::SelfOnly => selected.to_vec(),
            PlanClosureMode::All => all_nodes.to_vec(),
            PlanClosureMode::Upstream => self.walk(selected, true),
            PlanClosureMode::Downstream => self.walk(selected, false),
            PlanClosureMode::Connected => {
                let mut combined: BTreeSet<String> =
                    self.walk(selected, true).into_iter().collect();
                combined.extend(self.walk(selected, false));
                combined.into_iter().collect()
            }
        }
    }

    pub fn exact_input_names_for_transitive_upstream(
        &self,
        node_id: &str,
    ) -> BTreeMap<String, String> {
        let selected = vec![node_id.to_string()];
        let upstream: BTreeSet<String> = self
            .walk(&selected, true)
            .into_iter()
            .filter(|id| id != node_id)
            .collect();
        self.graph
            .dependencies_of(node_id)
            .into_iter()
            .filter(|edge| upstream.contains(&edge.to))
            .filter_map(|edge| {
                edge.input_name()
                    .map(|name| (edge.to.clone(), name.to_string()))
            })
            .collect()
    }

    fn walk(&self, selected: &[String], upstream: bool) -> Vec<String> {
        let mut result = BTreeSet::new();
        let mut stack = selected.to_vec();
        while let Some(node) = stack.pop() {
            if result.insert(node.clone()) {
                let next_edges = if upstream {
                    self.graph.dependencies_of(&node)
                } else {
                    self.graph.dependents_of(&node)
                };
                for edge in next_edges {
                    stack.push(if upstream {
                        edge.to.clone()
                    } else {
                        edge.from.clone()
                    });
                }
            }
        }
        result.into_iter().collect()
    }

    fn order_nodes(
        &self,
        nodes: &[String],
        mode: PlanOrderMode,
        stable_order: &[String],
    ) -> Result<Vec<String>, String> {
        let node_set: BTreeSet<&String> = nodes.iter().collect();
        if mode == PlanOrderMode::Stable {
            return Ok(stable_order
                .iter()
                .filter(|id| node_set.contains(*id))
                .cloned()
                .collect());
        }
        let mut in_degree: BTreeMap<String, usize> = nodes.iter().map(|n| (n.clone(), 0)).collect();
        let mut outgoing: BTreeMap<String, Vec<String>> =
            nodes.iter().map(|n| (n.clone(), Vec::new())).collect();
        for edge in self.graph.semantic_edges() {
            if node_set.contains(&edge.from) && node_set.contains(&edge.to) {
                outgoing
                    .entry(edge.to.clone())
                    .or_default()
                    .push(edge.from.clone());
                *in_degree.entry(edge.from.clone()).or_insert(0) += 1;
            }
        }
        let mut ready: Vec<String> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(id, _)| id.clone())
            .collect();
        self.sort_ready(&mut ready, stable_order);
        let mut result = Vec::new();
        while let Some(node) = ready.first().cloned() {
            ready.retain(|n| n != &node);
            result.push(node.clone());
            if let Some(consumers) = outgoing.get(&node) {
                for consumer in consumers {
                    if let Some(deg) = in_degree.get_mut(consumer) {
                        *deg -= 1;
                        if *deg == 0 {
                            ready.push(consumer.clone());
                        }
                    }
                }
            }
            self.sort_ready(&mut ready, stable_order);
        }
        if result.len() != nodes.len() {
            let unresolved: Vec<String> = nodes
                .iter()
                .filter(|n| !result.contains(n))
                .cloned()
                .collect();
            return Err(format!(
                "Cannot order Stitch DAG scope: cycle among {}",
                unresolved.join(", ")
            ));
        }
        if mode == PlanOrderMode::ConsumersFirst {
            result.reverse();
        }
        Ok(result)
    }

    fn sort_ready(&self, ready: &mut [String], stable_order: &[String]) {
        ready.sort_by(|a, b| {
            let layer_a = self.graph.node(a).and_then(|n| n.layer).unwrap_or(u32::MAX);
            let layer_b = self.graph.node(b).and_then(|n| n.layer).unwrap_or(u32::MAX);
            let pos_a = stable_order
                .iter()
                .position(|x| x == a)
                .unwrap_or(usize::MAX);
            let pos_b = stable_order
                .iter()
                .position(|x| x == b)
                .unwrap_or(usize::MAX);
            layer_a
                .cmp(&layer_b)
                .then_with(|| pos_a.cmp(&pos_b))
                .then_with(|| a.cmp(b))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::canonical::CanonicalWorkspaceGraph;
    use crate::graph::spec::{EdgeKind, EdgeSpec, NodeSpec, WorkspaceGraphDraft};
    use crate::graph::{NodeKind, RepoRole};

    fn node(id: &str, layer: u32) -> NodeSpec {
        NodeSpec {
            id: id.into(),
            path: id.into(),
            repo_url: None,
            kind: NodeKind::Unknown,
            role: RepoRole::Unknown,
            layer: Some(layer),
            is_root: false,
        }
    }
    fn graph() -> CanonicalWorkspaceGraph {
        let mut draft = WorkspaceGraphDraft::default();
        draft.nodes.insert("pins".into(), node("pins", 0));
        draft.nodes.insert("tools".into(), node("tools", 2));
        draft.nodes.insert("hosts".into(), node("hosts", 5));
        draft.edges.push(EdgeSpec {
            from: "tools".into(),
            to: "pins".into(),
            kind: EdgeKind::FlakeInput {
                input_name: "pin-input".into(),
                lock_file: "flake.lock".into(),
            },
        });
        draft.edges.push(EdgeSpec {
            from: "hosts".into(),
            to: "tools".into(),
            kind: EdgeKind::FlakeInput {
                input_name: "tools-input".into(),
                lock_file: "flake.lock".into(),
            },
        });
        CanonicalWorkspaceGraph::from_draft(draft).unwrap()
    }

    #[test]
    fn planner_orders_providers_before_consumers() {
        let graph = graph();
        let plan = DagPlanner::new(&graph)
            .plan(&DagPlanRequest {
                selection: PlanSelectionMode::All,
                explicit_nodes: Vec::new(),
                closure: PlanClosureMode::All,
                order: PlanOrderMode::ProvidersFirst,
                stable_order: vec!["hosts".into(), "tools".into(), "pins".into()],
            })
            .unwrap();
        assert_eq!(
            plan.nodes
                .iter()
                .map(|n| n.name.as_str())
                .collect::<Vec<_>>(),
            vec!["pins", "tools", "hosts"]
        );
    }

    #[test]
    fn planner_tracks_exact_direct_input_name() {
        let graph = graph();
        let names = DagPlanner::new(&graph).exact_input_names_for_transitive_upstream("hosts");
        assert_eq!(names.get("tools").map(String::as_str), Some("tools-input"));
    }
}
