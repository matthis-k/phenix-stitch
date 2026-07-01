use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::graph::{NodeKind, RepoRole, WorkspaceDag, WorkspaceEdge, WorkspaceNode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    pub nodes: Vec<String>,
    pub edge: Option<WorkspaceEdge>,
}

impl GraphDiagnostic {
    pub fn error(code: &str, message: String, nodes: Vec<String>) -> Self {
        GraphDiagnostic {
            severity: DiagnosticSeverity::Error,
            code: code.to_string(),
            message,
            nodes,
            edge: None,
        }
    }

    pub fn warn(code: &str, message: String, nodes: Vec<String>) -> Self {
        GraphDiagnostic {
            severity: DiagnosticSeverity::Warning,
            code: code.to_string(),
            message,
            nodes,
            edge: None,
        }
    }

    pub fn info(code: &str, message: String, nodes: Vec<String>) -> Self {
        GraphDiagnostic {
            severity: DiagnosticSeverity::Info,
            code: code.to_string(),
            message,
            nodes,
            edge: None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ValidateOptions {
    pub strict: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphValidationReport {
    pub valid: bool,
    pub diagnostics: Vec<GraphDiagnostic>,
    pub node_count: usize,
    pub edge_count: usize,
}

pub fn validate_graph(graph: &WorkspaceDag, opts: &ValidateOptions) -> GraphValidationReport {
    let mut diagnostics = Vec::new();

    // 1. Check for unknowns in edges
    for edge in &graph.edges {
        if !graph.nodes.contains_key(&edge.from) {
            diagnostics.push(GraphDiagnostic::error(
                "unknown_source_node",
                format!("edge source '{}' is not a known workspace node", edge.from),
                vec![edge.from.clone()],
            ));
        }
        if !graph.nodes.contains_key(&edge.to) {
            diagnostics.push(GraphDiagnostic::error(
                "unknown_target_node",
                format!("edge target '{}' is not a known workspace node", edge.to),
                vec![edge.to.clone()],
            ));
        }
    }

    // 2. Cycle detection
    if let Some(cycle) = find_cycle(graph) {
        diagnostics.push(GraphDiagnostic::error(
            "cycle_detected",
            format!("cycle detected: {}", cycle.join(" -> ")),
            cycle,
        ));
    }

    // 3. Layer rule: consumer layer must be > provider layer (errors by default)
    for edge in &graph.edges {
        let from_node = match graph.nodes.get(&edge.from) {
            Some(n) => n,
            None => continue,
        };
        let to_node = match graph.nodes.get(&edge.to) {
            Some(n) => n,
            None => continue,
        };

        if from_node.is_root {
            continue;
        }

        if let (Some(from_layer), Some(to_layer)) = (from_node.layer, to_node.layer) {
            if to_layer >= from_layer {
                let msg = format!(
                    "layer violation: '{}' (layer {}) -> '{}' (layer {}): dependencies must point to lower layers",
                    edge.from, from_layer, edge.to, to_layer
                );
                diagnostics.push(GraphDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    code: "layer_violation".to_string(),
                    message: msg,
                    nodes: vec![edge.from.clone(), edge.to.clone()],
                    edge: Some(edge.clone()),
                });
            }
        } else {
            let sev = if opts.strict {
                DiagnosticSeverity::Warning
            } else {
                DiagnosticSeverity::Info
            };
            diagnostics.push(GraphDiagnostic {
                severity: sev,
                code: "missing_layer".to_string(),
                message: format!(
                    "edge '{}' -> '{}': one or both nodes have no layer assigned",
                    edge.from, edge.to
                ),
                nodes: vec![edge.from.clone(), edge.to.clone()],
                edge: Some(edge.clone()),
            });
        }
    }

    // 4. Root dependency rule: no non-root node should depend on root
    for edge in &graph.edges {
        let to_node = match graph.nodes.get(&edge.to) {
            Some(n) => n,
            None => continue,
        };
        if to_node.is_root {
            diagnostics.push(GraphDiagnostic::error(
                "root_dependency_violation",
                format!(
                    "'{}' depends on root node '{}': non-root nodes must not depend on the workspace root",
                    edge.from, edge.to
                ),
                vec![edge.from.clone(), edge.to.clone()],
            ));
        }
    }

    // 5. Hard role rules
    for edge in &graph.edges {
        let from_node = match graph.nodes.get(&edge.from) {
            Some(n) => n,
            None => continue,
        };
        let to_node = match graph.nodes.get(&edge.to) {
            Some(n) => n,
            None => continue,
        };

        validate_role_edge(from_node, to_node, edge, &mut diagnostics);
    }

    // 6. Folder prefix layer check
    for node in graph.nodes.values() {
        if node.role != RepoRole::Root {
            if let (Some(config_layer), Some(path_layer)) = (node.layer, folder_layer(&node.path)) {
                if config_layer != path_layer {
                    diagnostics.push(GraphDiagnostic::error(
                        "path_layer_mismatch",
                        format!(
                            "'{}' configured layer {} but path '{}' indicates layer {}",
                            node.id,
                            config_layer,
                            node.path.display(),
                            path_layer
                        ),
                        vec![node.id.clone()],
                    ));
                }
            }
        }
    }

    // 7. Duplicate edge warnings
    let mut seen_edges: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for edge in &graph.edges {
        let key = (edge.from.clone(), edge.to.clone());
        if !seen_edges.insert(key) {
            diagnostics.push(GraphDiagnostic::warn(
                "duplicate_edge",
                format!("duplicate edge: '{}' -> '{}'", edge.from, edge.to),
                vec![edge.from.clone(), edge.to.clone()],
            ));
        }
    }

    // 8. External conflict warnings
    if !graph.external_inputs.is_empty() {
        let mut by_name: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        for ext in &graph.external_inputs {
            by_name
                .entry(ext.input_name.clone())
                .or_default()
                .push(ext.owner_node.as_str());
        }
        for (input_name, owners) in &by_name {
            if owners.len() > 1 {
                diagnostics.push(GraphDiagnostic::warn(
                    "external_input_multi_owner",
                    format!(
                        "external input '{input_name}' referenced by multiple nodes: {}",
                        owners.join(", ")
                    ),
                    owners.iter().map(|s| s.to_string()).collect(),
                ));
            }
        }
    }

    // Merge existing diagnostics from graph construction
    let all_diagnostics: Vec<GraphDiagnostic> = graph
        .diagnostics
        .clone()
        .into_iter()
        .chain(diagnostics)
        .collect();

    let has_errors = all_diagnostics
        .iter()
        .any(|d| d.severity == DiagnosticSeverity::Error);

    GraphValidationReport {
        valid: !has_errors,
        diagnostics: all_diagnostics,
        node_count: graph.nodes.len(),
        edge_count: graph.edges.len(),
    }
}

pub fn validate_canonical_graph(
    graph: &crate::graph::CanonicalWorkspaceGraph,
    opts: &ValidateOptions,
) -> GraphValidationReport {
    validate_graph(&graph.to_legacy_dag(), opts)
}

fn validate_role_edge(
    from: &WorkspaceNode,
    to: &WorkspaceNode,
    _edge: &WorkspaceEdge,
    diagnostics: &mut Vec<GraphDiagnostic>,
) {
    // Only check non-root edges
    if from.is_root {
        return;
    }

    // Root dependency: already checked above
    if to.is_root {
        return;
    }

    // Producer depends on producer
    if from.role == RepoRole::Producer && to.role == RepoRole::Producer {
        diagnostics.push(GraphDiagnostic::error(
            "producer_depends_on_producer",
            format!(
                "'{}' is a producer and may not depend on producer '{}'; use protocols or integrations",
                from.id, to.id
            ),
            vec![from.id.clone(), to.id.clone()],
        ));
    }

    // Producer depends on pkgs-aggregator
    if from.role == RepoRole::Producer && to.role == RepoRole::PkgsAggregator {
        diagnostics.push(GraphDiagnostic::error(
            "producer_depends_on_pkgs_aggregator",
            format!(
                "'{}' is a producer and may not depend on package aggregator '{}'; use pkgs-base",
                from.id, to.id
            ),
            vec![from.id.clone(), to.id.clone()],
        ));
    }

    // Provider depends on consumer (from old model - keep for compat)
    if from.kind.is_provider() && to.kind.is_consumer() {
        diagnostics.push(GraphDiagnostic::error(
            "provider_depends_on_consumer",
            format!(
                "'{}' ({}) depends on '{}' ({}): providers must not depend on consumers",
                from.id,
                node_kind_name(&from.kind),
                to.id,
                node_kind_name(&to.kind)
            ),
            vec![from.id.clone(), to.id.clone()],
        ));
    }
}

fn folder_layer(path: &Path) -> Option<u32> {
    let mut components = path.components().peekable();

    while let Some(component) = components.next() {
        let c = component.as_os_str().to_string_lossy();
        if c == "flakes" {
            let layer_component = components.next()?.as_os_str().to_string_lossy();
            let number = layer_component.split('-').next()?;
            return number.parse::<u32>().ok();
        }
    }

    None
}

fn node_kind_name(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Pins => "pins",
        NodeKind::PackageProvider => "packageProvider",
        NodeKind::ToolProvider => "toolProvider",
        NodeKind::ShellProvider => "shellProvider",
        NodeKind::DesktopProvider => "desktopProvider",
        NodeKind::HostConsumer => "hostConsumer",
        NodeKind::WorkspaceRoot => "workspaceRoot",
        NodeKind::External => "external",
        NodeKind::Unknown => "unknown",
    }
}

enum Mark {
    Temporary,
    Permanent,
}

fn find_cycle(graph: &WorkspaceDag) -> Option<Vec<String>> {
    let mut marks: BTreeMap<String, Mark> = BTreeMap::new();
    let mut stack: Vec<String> = Vec::new();

    for node_id in graph.nodes.keys() {
        if !matches!(marks.get(node_id), Some(Mark::Permanent)) {
            if let Some(cycle) = visit(node_id, graph, &mut marks, &mut stack) {
                return Some(cycle);
            }
        }
    }

    None
}

fn visit(
    node: &str,
    graph: &WorkspaceDag,
    marks: &mut BTreeMap<String, Mark>,
    stack: &mut Vec<String>,
) -> Option<Vec<String>> {
    if matches!(marks.get(node), Some(Mark::Temporary)) {
        let start = stack.iter().position(|n| n == node).unwrap_or(0);
        let mut cycle = stack[start..].to_vec();
        cycle.push(node.to_string());
        return Some(cycle);
    }

    if matches!(marks.get(node), Some(Mark::Permanent)) {
        return None;
    }

    marks.insert(node.to_string(), Mark::Temporary);
    stack.push(node.to_string());

    for edge in graph.edges.iter().filter(|e| e.from == node) {
        if let Some(cycle) = visit(&edge.to, graph, marks, stack) {
            return Some(cycle);
        }
    }

    stack.pop();
    marks.insert(node.to_string(), Mark::Permanent);
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeReason, NodeKind, RepoRole};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn make_node(
        id: &str,
        kind: NodeKind,
        role: RepoRole,
        layer: Option<u32>,
        is_root: bool,
    ) -> WorkspaceNode {
        WorkspaceNode {
            id: id.to_string(),
            path: PathBuf::new(),
            repo_url: None,
            kind,
            role,
            layer,
            is_root,
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

    fn make_graph(nodes: Vec<WorkspaceNode>, edges: Vec<WorkspaceEdge>) -> WorkspaceDag {
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

    fn make_node_old(id: &str, kind: NodeKind, layer: Option<u32>, is_root: bool) -> WorkspaceNode {
        let role = match kind {
            NodeKind::Pins => RepoRole::Pins,
            NodeKind::PackageProvider => RepoRole::PkgsAggregator,
            NodeKind::ToolProvider => RepoRole::Producer,
            NodeKind::ShellProvider => RepoRole::Producer,
            NodeKind::DesktopProvider => RepoRole::Consumer,
            NodeKind::HostConsumer => RepoRole::Consumer,
            NodeKind::WorkspaceRoot => RepoRole::Root,
            NodeKind::External => RepoRole::External,
            NodeKind::Unknown => RepoRole::Unknown,
        };
        make_node(id, kind, role, layer, is_root)
    }

    #[test]
    fn test_cycle_detection() {
        let nodes = vec![
            make_node_old("a", NodeKind::Unknown, None, false),
            make_node_old("b", NodeKind::Unknown, None, false),
            make_node_old("c", NodeKind::Unknown, None, false),
        ];
        let edges = vec![
            make_edge("a", "b"),
            make_edge("b", "c"),
            make_edge("c", "a"),
        ];
        let graph = make_graph(nodes, edges);
        let report = validate_graph(&graph, &ValidateOptions::default());
        assert!(!report.valid);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "cycle_detected"));
    }

    #[test]
    fn test_layer_violation() {
        let nodes = vec![
            make_node_old("pins", NodeKind::Pins, Some(0), false),
            make_node_old("hosts", NodeKind::HostConsumer, Some(5), false),
        ];
        let edges = vec![make_edge("pins", "hosts")];
        let graph = make_graph(nodes, edges);
        let report = validate_graph(&graph, &ValidateOptions::default());
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "layer_violation"));
    }

    #[test]
    fn test_layer_ok() {
        let nodes = vec![
            make_node_old("hosts", NodeKind::HostConsumer, Some(5), false),
            make_node_old("pins", NodeKind::Pins, Some(0), false),
        ];
        let edges = vec![make_edge("hosts", "pins")];
        let graph = make_graph(nodes, edges);
        let report = validate_graph(&graph, &ValidateOptions::default());
        assert!(!report
            .diagnostics
            .iter()
            .any(|d| d.code == "layer_violation"));
    }

    #[test]
    fn test_no_layer_violation() {
        let nodes = vec![
            make_node_old("hosts", NodeKind::HostConsumer, Some(5), false),
            make_node_old("pins", NodeKind::Pins, Some(0), false),
        ];
        let edges = vec![make_edge("hosts", "pins")];
        let graph = make_graph(nodes, edges);
        let report = validate_graph(&graph, &ValidateOptions::default());
        assert!(!report
            .diagnostics
            .iter()
            .any(|d| d.code == "layer_violation"));
        assert!(report.valid);
    }

    #[test]
    fn test_root_dependency_violation() {
        let nodes = vec![
            make_node_old("phenix-tools", NodeKind::ToolProvider, Some(2), false),
            make_node_old("phenix", NodeKind::WorkspaceRoot, Some(6), true),
        ];
        let edges = vec![make_edge("phenix-tools", "phenix")];
        let graph = make_graph(nodes, edges);
        let report = validate_graph(&graph, &ValidateOptions::default());
        assert!(!report.valid);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "root_dependency_violation"));
    }

    #[test]
    fn test_producer_depends_on_producer() {
        let nodes = vec![
            make_node(
                "tools",
                NodeKind::ToolProvider,
                RepoRole::Producer,
                Some(2),
                false,
            ),
            make_node(
                "nvim",
                NodeKind::ToolProvider,
                RepoRole::Producer,
                Some(2),
                false,
            ),
            make_node("pins", NodeKind::Pins, RepoRole::Pins, Some(0), false),
        ];
        let edges = vec![make_edge("tools", "pins"), make_edge("tools", "nvim")];
        let graph = make_graph(nodes, edges);
        let report = validate_graph(&graph, &ValidateOptions::default());
        assert!(!report.valid);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "producer_depends_on_producer"));
    }

    #[test]
    fn test_producer_depends_on_pkgs_aggregator() {
        let nodes = vec![
            make_node(
                "tools",
                NodeKind::ToolProvider,
                RepoRole::Producer,
                Some(2),
                false,
            ),
            make_node(
                "pkgs",
                NodeKind::PackageProvider,
                RepoRole::PkgsAggregator,
                Some(4),
                false,
            ),
        ];
        let edges = vec![make_edge("tools", "pkgs")];
        let graph = make_graph(nodes, edges);
        let report = validate_graph(&graph, &ValidateOptions::default());
        assert!(!report.valid);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "producer_depends_on_pkgs_aggregator"));
    }

    #[test]
    fn test_valid_graph() {
        let nodes = vec![
            make_node_old("phenix-pins", NodeKind::Pins, Some(0), false),
            make_node(
                "phenix-packages",
                NodeKind::PackageProvider,
                RepoRole::PkgsAggregator,
                Some(4),
                false,
            ),
            make_node_old("phenix-tools", NodeKind::ToolProvider, Some(2), false),
            make_node_old("phenix-hosts", NodeKind::HostConsumer, Some(5), false),
            make_node_old("phenix", NodeKind::WorkspaceRoot, Some(6), true),
        ];
        let edges = vec![
            make_edge("phenix-packages", "phenix-pins"),
            make_edge("phenix-tools", "phenix-pins"),
            make_edge("phenix-hosts", "phenix-packages"),
            make_edge("phenix-hosts", "phenix-tools"),
            make_edge("phenix", "phenix-packages"),
            make_edge("phenix", "phenix-tools"),
            make_edge("phenix", "phenix-hosts"),
        ];
        let graph = make_graph(nodes, edges);
        let report = validate_graph(&graph, &ValidateOptions::default());
        assert!(report.valid);
    }

    #[test]
    fn test_validate_canonical_graph_preserves_rule_coverage() {
        let nodes = vec![
            make_node_old("hosts", NodeKind::HostConsumer, Some(5), false),
            make_node_old("pins", NodeKind::Pins, Some(0), false),
        ];
        let graph = make_graph(nodes, vec![make_edge("hosts", "pins")]);
        let canonical = crate::graph::CanonicalWorkspaceGraph::from_legacy(graph).unwrap();
        let report = validate_canonical_graph(&canonical, &ValidateOptions::default());
        assert!(report.valid);
        assert_eq!(report.edge_count, 1);
    }

    #[test]
    fn test_cycle_report_string() {
        let nodes = vec![
            make_node_old("a", NodeKind::Unknown, None, false),
            make_node_old("b", NodeKind::Unknown, None, false),
        ];
        let edges = vec![make_edge("a", "b"), make_edge("b", "a")];
        let graph = make_graph(nodes, edges);
        let report = validate_graph(&graph, &ValidateOptions::default());
        let cycle_diag = report
            .diagnostics
            .iter()
            .find(|d| d.code == "cycle_detected")
            .unwrap();
        assert!(cycle_diag.message.contains("a"));
        assert!(cycle_diag.message.contains("b"));
    }

    #[test]
    fn test_folder_layer() {
        let p = Path::new("flakes/02-producers/phenix-tools");
        assert_eq!(folder_layer(p), Some(2));

        let p = Path::new("/abs/flakes/05-consumers/phenix-hosts");
        assert_eq!(folder_layer(p), Some(5));

        let p = Path::new("some/other/path");
        assert_eq!(folder_layer(p), None);
    }
}
