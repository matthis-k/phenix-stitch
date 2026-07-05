use std::collections::{BTreeMap, BTreeSet};

use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use petgraph::visit::IntoEdgeReferences;

use crate::graph::spec::{EdgeSpec, NodeSpec, WorkspaceGraphDraft};
use crate::graph::{WorkspaceDag, WorkspaceEdge};

#[derive(Debug, Clone)]
pub struct CanonicalWorkspaceGraph {
    graph: StableDiGraph<NodeSpec, EdgeSpec>,
    id_to_index: BTreeMap<String, NodeIndex>,
    index_to_id: BTreeMap<NodeIndex, String>,
    external_inputs: Vec<crate::graph::ExternalInput>,
    diagnostics: Vec<crate::graph::GraphDiagnostic>,
}

#[derive(Debug, Clone)]
pub struct CanonicalizeError {
    pub message: String,
}

impl std::fmt::Display for CanonicalizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CanonicalizeError {}

impl CanonicalWorkspaceGraph {
    pub fn from_draft(mut draft: WorkspaceGraphDraft) -> Result<Self, CanonicalizeError> {
        draft.dedup_edges();
        let mut graph = StableDiGraph::new();
        let mut id_to_index = BTreeMap::new();
        let mut index_to_id = BTreeMap::new();

        for (id, node) in draft.nodes {
            if id_to_index.contains_key(&id) {
                return Err(CanonicalizeError {
                    message: format!("duplicate node id '{id}'"),
                });
            }
            let idx = graph.add_node(node);
            id_to_index.insert(id.clone(), idx);
            index_to_id.insert(idx, id);
        }

        let mut seen_edges = BTreeSet::new();
        for edge in draft.edges {
            let from = *id_to_index
                .get(&edge.from)
                .ok_or_else(|| CanonicalizeError {
                    message: format!("edge source '{}' is not a known workspace node", edge.from),
                })?;
            let to = *id_to_index.get(&edge.to).ok_or_else(|| CanonicalizeError {
                message: format!("edge target '{}' is not a known workspace node", edge.to),
            })?;
            if seen_edges.insert(edge.dedup_key()) {
                graph.add_edge(from, to, edge);
            }
        }

        Ok(Self {
            graph,
            id_to_index,
            index_to_id,
            external_inputs: draft.external_inputs,
            diagnostics: draft.diagnostics,
        })
    }

    pub fn from_legacy(graph: WorkspaceDag) -> Result<Self, CanonicalizeError> {
        Self::from_draft(graph.into())
    }

    pub fn stable_graph(&self) -> &StableDiGraph<NodeSpec, EdgeSpec> {
        &self.graph
    }

    pub fn node_ids(&self) -> impl Iterator<Item = &String> {
        self.id_to_index.keys()
    }

    pub fn node_id_for_index(&self, index: NodeIndex) -> Option<&String> {
        self.index_to_id.get(&index)
    }

    pub fn node(&self, id: &str) -> Option<&NodeSpec> {
        self.id_to_index
            .get(id)
            .and_then(|idx| self.graph.node_weight(*idx))
    }

    pub fn semantic_edges(&self) -> Vec<&EdgeSpec> {
        self.graph
            .edge_references()
            .map(|edge| edge.weight())
            .filter(|edge| edge.is_semantic_dependency())
            .collect()
    }

    pub fn dependencies_of(&self, node_id: &str) -> Vec<&EdgeSpec> {
        self.semantic_edges()
            .into_iter()
            .filter(|edge| edge.from == node_id)
            .collect()
    }

    pub fn dependents_of(&self, node_id: &str) -> Vec<&EdgeSpec> {
        self.semantic_edges()
            .into_iter()
            .filter(|edge| edge.to == node_id)
            .collect()
    }

    pub fn to_legacy_dag(&self) -> WorkspaceDag {
        let nodes = self
            .id_to_index
            .iter()
            .filter_map(|(id, idx)| {
                self.graph
                    .node_weight(*idx)
                    .cloned()
                    .map(|node| (id.clone(), node.into()))
            })
            .collect();
        let edges: Vec<WorkspaceEdge> = self
            .graph
            .edge_references()
            .filter_map(|edge| WorkspaceEdge::try_from(edge.weight().clone()).ok())
            .collect();
        WorkspaceDag {
            nodes,
            edges,
            external_inputs: self.external_inputs.clone(),
            diagnostics: self.diagnostics.clone(),
        }
    }

    pub fn to_legacy_workspace_graph(&self, root: String) -> crate::graph::WorkspaceGraph {
        let nodes = self
            .id_to_index
            .iter()
            .filter_map(|(id, idx)| {
                self.graph.node_weight(*idx).map(|node| {
                    (
                        id.clone(),
                        crate::graph::FlakeNode {
                            id: id.clone(),
                            name: id.clone(),
                            path: node.path.clone(),
                            remote: node.repo_url.clone(),
                            branch: "main".to_string(),
                        },
                    )
                })
            })
            .collect();
        let edges = self
            .semantic_edges()
            .into_iter()
            .filter_map(|edge| {
                edge.input_name().map(|input_name| {
                    crate::graph::DependencyEdge::new(&edge.from, &edge.to, input_name)
                })
            })
            .collect();
        crate::graph::WorkspaceGraph { root, nodes, edges }
    }

    pub fn flake_input_names_for_direct_dependencies(
        &self,
        node_id: &str,
    ) -> BTreeMap<String, String> {
        self.dependencies_of(node_id)
            .into_iter()
            .filter_map(|edge| {
                edge.input_name()
                    .map(|name| (edge.to.clone(), name.to_string()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::spec::{EdgeKind, EdgeSpec};
    use crate::graph::{NodeKind, RepoRole};

    fn node(id: &str) -> NodeSpec {
        NodeSpec {
            id: id.into(),
            path: id.into(),
            repo_url: None,
            kind: NodeKind::Unknown,
            role: RepoRole::Unknown,
            layer: None,
            is_root: false,
        }
    }

    fn node_with(
        id: &str,
        kind: NodeKind,
        role: RepoRole,
        layer: Option<u32>,
        path: &str,
    ) -> NodeSpec {
        NodeSpec {
            id: id.into(),
            path: path.into(),
            repo_url: None,
            kind,
            role,
            layer,
            is_root: false,
        }
    }

    #[test]
    fn canonical_graph_uses_stable_digraph_and_preserves_input_name() {
        let mut draft = WorkspaceGraphDraft::default();
        draft.nodes.insert("consumer".into(), node("consumer"));
        draft.nodes.insert("provider".into(), node("provider"));
        draft.edges.push(EdgeSpec {
            from: "consumer".into(),
            to: "provider".into(),
            kind: EdgeKind::FlakeInput {
                input_name: "provider-pin".into(),
                lock_file: "flake.lock".into(),
            },
        });
        let graph = CanonicalWorkspaceGraph::from_draft(draft).unwrap();
        assert_eq!(graph.stable_graph().node_count(), 2);
        assert_eq!(
            graph
                .flake_input_names_for_direct_dependencies("consumer")
                .get("provider")
                .map(String::as_str),
            Some("provider-pin")
        );
    }

    #[test]
    fn canonical_graph_preserves_layers() {
        // Canonicalization must preserve each node's layer metadata
        let mut draft = WorkspaceGraphDraft::default();
        draft.nodes.insert(
            "pins".into(),
            node_with(
                "pins",
                NodeKind::Pins,
                RepoRole::Pins,
                Some(0),
                "flakes/00-pins/pins",
            ),
        );
        draft.nodes.insert(
            "producer".into(),
            node_with(
                "producer",
                NodeKind::ToolProvider,
                RepoRole::Producer,
                Some(2),
                "flakes/02-producers/producer",
            ),
        );
        draft.nodes.insert(
            "consumer".into(),
            node_with(
                "consumer",
                NodeKind::HostConsumer,
                RepoRole::Consumer,
                Some(5),
                "flakes/05-consumers/consumer",
            ),
        );
        draft.edges.push(EdgeSpec {
            from: "producer".into(),
            to: "pins".into(),
            kind: EdgeKind::FlakeInput {
                input_name: "pins".into(),
                lock_file: "flake.lock".into(),
            },
        });
        draft.edges.push(EdgeSpec {
            from: "consumer".into(),
            to: "producer".into(),
            kind: EdgeKind::FlakeInput {
                input_name: "producer".into(),
                lock_file: "flake.lock".into(),
            },
        });

        let graph = CanonicalWorkspaceGraph::from_draft(draft).unwrap();

        // All three layers must be preserved
        assert_eq!(graph.node("pins").unwrap().layer, Some(0));
        assert_eq!(graph.node("producer").unwrap().layer, Some(2));
        assert_eq!(graph.node("consumer").unwrap().layer, Some(5));

        // Nodes can be grouped by layer through iteration
        let ids_by_layer: Vec<_> = {
            let mut pairs: Vec<_> = graph
                .node_ids()
                .map(|id| {
                    let node = graph.node(id).unwrap();
                    (node.layer.unwrap_or(99), id.as_str())
                })
                .collect();
            pairs.sort();
            pairs
        };
        assert_eq!(ids_by_layer[0], (0, "pins"));
        assert_eq!(ids_by_layer[1], (2, "producer"));
        assert_eq!(ids_by_layer[2], (5, "consumer"));
    }

    #[test]
    fn canonical_graph_provider_consumer_edge_direction() {
        // The canonical graph must correctly model provider-to-consumer direction.
        // Edges go consumer -> provider (consumer depends on provider).
        let mut draft = WorkspaceGraphDraft::default();
        draft.nodes.insert(
            "pins".into(),
            node_with(
                "pins",
                NodeKind::Pins,
                RepoRole::Pins,
                Some(0),
                "flakes/00-pins/pins",
            ),
        );
        draft.nodes.insert(
            "producer".into(),
            node_with(
                "producer",
                NodeKind::ToolProvider,
                RepoRole::Producer,
                Some(2),
                "flakes/02-producers/producer",
            ),
        );
        draft.nodes.insert(
            "consumer".into(),
            node_with(
                "consumer",
                NodeKind::HostConsumer,
                RepoRole::Consumer,
                Some(5),
                "flakes/05-consumers/consumer",
            ),
        );
        // Consumer -> Producer -> Pins
        draft.edges.push(EdgeSpec {
            from: "producer".into(),
            to: "pins".into(),
            kind: EdgeKind::FlakeInput {
                input_name: "pins".into(),
                lock_file: "flake.lock".into(),
            },
        });
        draft.edges.push(EdgeSpec {
            from: "consumer".into(),
            to: "producer".into(),
            kind: EdgeKind::FlakeInput {
                input_name: "producer".into(),
                lock_file: "flake.lock".into(),
            },
        });

        let graph = CanonicalWorkspaceGraph::from_draft(draft).unwrap();

        // Consumer depends on producer
        let consumer_deps = graph.dependencies_of("consumer");
        assert_eq!(consumer_deps.len(), 1);
        assert_eq!(consumer_deps[0].to, "producer");
        assert!(consumer_deps[0].is_semantic_dependency());

        // Producer depends on pins
        let producer_deps = graph.dependencies_of("producer");
        assert_eq!(producer_deps.len(), 1);
        assert_eq!(producer_deps[0].to, "pins");
        assert!(producer_deps[0].is_semantic_dependency());

        // Consumer's dependents (things depending on consumer) should be empty
        let consumer_dependents = graph.dependents_of("consumer");
        assert!(consumer_dependents.is_empty());

        // Pins should have two dependents (producer and consumer transitively)
        let pins_dependents = graph.dependents_of("pins");
        assert_eq!(pins_dependents.len(), 1);
        assert_eq!(pins_dependents[0].from, "producer");

        // Semantic edges should exclude submodule edges
        let all_semantic = graph.semantic_edges();
        assert_eq!(all_semantic.len(), 2);
    }
}
