use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::graph::{EdgeSpec, NodeKind, NodeSpec, RepoRole, WorkspaceGraphDraft};

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
    pub edge: Option<EdgeSpec>,
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

fn validate_snapshot(graph: &WorkspaceGraphDraft, opts: &ValidateOptions) -> GraphValidationReport {
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

    // 9. URL mismatch check: repo URL should match the node id
    for node in graph.nodes.values() {
        if let Some(ref repo_url) = node.repo_url {
            if !repo_url.is_empty() {
                let expected_name = url_repo_name(repo_url).unwrap_or("");
                if !expected_name.is_empty() && expected_name != node.id {
                    diagnostics.push(GraphDiagnostic::error(
                        "url_mismatch",
                        format!(
                            "'{}' has repo URL '{}' which resolves to name '{}', expected '{}'",
                            node.id, repo_url, expected_name, node.id
                        ),
                        vec![node.id.clone()],
                    ));
                }
            }
        }
    }

    // 10. Missing repo path check: warn when a configured path does not exist on disk
    for node in graph.nodes.values() {
        let path_str = node.path.to_string_lossy();
        if !path_str.is_empty() && !node.path.exists() {
            diagnostics.push(GraphDiagnostic::warn(
                "missing_repo_path",
                format!(
                    "'{}' has path '{}' which does not exist on disk",
                    node.id,
                    node.path.display()
                ),
                vec![node.id.clone()],
            ));
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

pub fn validate_snapshot(
    graph: &crate::graph::CanonicalWorkspaceGraph,
    opts: &ValidateOptions,
) -> GraphValidationReport {
    validate_snapshot(&graph.to_snapshot(), opts)
}

fn validate_role_edge(
    from: &NodeSpec,
    to: &NodeSpec,
    _edge: &EdgeSpec,
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

/// Extract the repository name from a git remote URL.
///
/// Handles HTTPS-style (`https://github.com/org/repo.git` → `repo`),
/// SSH-style (`git@github.com:org/repo.git` → `repo`),
/// and plain paths.
fn url_repo_name(url: &str) -> Option<&str> {
    let url = url.strip_suffix(".git").unwrap_or(url);
    // SSH-style: git@github.com:org/repo
    if let Some(idx) = url.find(':') {
        let after = &url[idx + 1..];
        return after.split('/').next_back();
    }
    // HTTPS-style or path-style
    url.split('/').next_back().filter(|s| !s.is_empty())
}

enum Mark {
    Temporary,
    Permanent,
}

fn find_cycle(graph: &WorkspaceGraphDraft) -> Option<Vec<String>> {
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
    graph: &WorkspaceGraphDraft,
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
    use crate::graph::{EdgeKind, NodeKind, RepoRole};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn make_node(
        id: &str,
        kind: NodeKind,
        role: RepoRole,
        layer: Option<u32>,
        is_root: bool,
    ) -> NodeSpec {
        NodeSpec {
            id: id.to_string(),
            path: PathBuf::new(),
            repo_url: None,
            kind,
            role,
            layer,
            is_root,
        }
    }

    fn make_edge(from: &str, to: &str) -> EdgeSpec {
        EdgeSpec {
            from: from.to_string(),
            to: to.to_string(),
            kind: EdgeKind::Manual {
                source_file: PathBuf::from("test"),
            },
        }
    }

    fn make_graph(nodes: Vec<NodeSpec>, edges: Vec<EdgeSpec>) -> WorkspaceGraphDraft {
        let mut node_map = BTreeMap::new();
        for n in nodes {
            node_map.insert(n.id.clone(), n);
        }
        WorkspaceGraphDraft {
            nodes: node_map,
            edges,
            external_inputs: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn make_node_old(id: &str, kind: NodeKind, layer: Option<u32>, is_root: bool) -> NodeSpec {
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
        let report = validate_snapshot(&graph, &ValidateOptions::default());
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
        let report = validate_snapshot(&graph, &ValidateOptions::default());
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
        let report = validate_snapshot(&graph, &ValidateOptions::default());
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
        let report = validate_snapshot(&graph, &ValidateOptions::default());
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
        let report = validate_snapshot(&graph, &ValidateOptions::default());
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
        let report = validate_snapshot(&graph, &ValidateOptions::default());
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
        let report = validate_snapshot(&graph, &ValidateOptions::default());
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
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(report.valid);
    }

    #[test]
    fn test_validate_graph_preserves_rule_coverage() {
        let nodes = vec![
            make_node_old("hosts", NodeKind::HostConsumer, Some(5), false),
            make_node_old("pins", NodeKind::Pins, Some(0), false),
        ];
        let graph = make_graph(nodes, vec![make_edge("hosts", "pins")]);
        let canonical = crate::graph::CanonicalWorkspaceGraph::from_draft(graph).unwrap();
        let report = validate_graph(&canonical, &ValidateOptions::default());
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
        let report = validate_snapshot(&graph, &ValidateOptions::default());
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

    // -----------------------------------------------------------------------
    // Path / layer mismatch tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_path_layer_mismatch() {
        // Repo with layer=2 but path lives in flakes/05-consumers/
        let nodes = vec![
            make_node(
                "phenix-host",
                NodeKind::HostConsumer,
                RepoRole::Consumer,
                Some(2),
                false,
            ),
            make_node("pins", NodeKind::Pins, RepoRole::Pins, Some(0), false),
        ];
        // Assign a path that indicates layer 5
        let mut node_map = BTreeMap::new();
        for mut n in nodes {
            if n.id == "phenix-host" {
                n.path = PathBuf::from("flakes/05-consumers/phenix-host");
            }
            node_map.insert(n.id.clone(), n);
        }
        let edges = vec![make_edge("phenix-host", "pins")];
        let graph = WorkspaceGraphDraft {
            nodes: node_map,
            edges,
            external_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(!report.valid);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "path_layer_mismatch"));
    }

    #[test]
    fn test_path_layer_match_ok() {
        // Repo with layer=5 and path in flakes/05-consumers/ — should pass
        let nodes = vec![
            make_node(
                "phenix-host",
                NodeKind::HostConsumer,
                RepoRole::Consumer,
                Some(5),
                false,
            ),
            make_node("pins", NodeKind::Pins, RepoRole::Pins, Some(0), false),
        ];
        let mut node_map = BTreeMap::new();
        for mut n in nodes {
            if n.id == "phenix-host" {
                n.path = PathBuf::from("flakes/05-consumers/phenix-host");
            }
            node_map.insert(n.id.clone(), n);
        }
        let edges = vec![make_edge("phenix-host", "pins")];
        let graph = WorkspaceGraphDraft {
            nodes: node_map,
            edges,
            external_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(report.valid);
        assert!(!report
            .diagnostics
            .iter()
            .any(|d| d.code == "path_layer_mismatch"));
    }

    #[test]
    fn test_path_layer_mismatch_ignores_root() {
        // Root nodes should be exempt from path/layer checks
        let nodes = vec![make_node(
            "phenix",
            NodeKind::WorkspaceRoot,
            RepoRole::Root,
            Some(6),
            true,
        )];
        let mut node_map = BTreeMap::new();
        for mut n in nodes {
            n.path = PathBuf::from("flakes/00-pins/phenix"); // non-root layer in path
            node_map.insert(n.id.clone(), n);
        }
        let graph = WorkspaceGraphDraft {
            nodes: node_map,
            edges: Vec::new(),
            external_inputs: Vec::new(),
            diagnostics: Vec::new(),
        };
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        // Root should not trigger path_layer_mismatch
        assert!(!report
            .diagnostics
            .iter()
            .any(|d| d.code == "path_layer_mismatch"));
        // And the graph should be valid
        assert!(report.valid);
    }

    // -----------------------------------------------------------------------
    // Same-layer dependency tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_same_layer_violation_detected() {
        // Two repos at the same layer (layer=2) depending on each other
        // internally: both edges should fail layer_violation
        let nodes = vec![
            make_node_old("tools-a", NodeKind::ToolProvider, Some(2), false),
            make_node_old("tools-b", NodeKind::ToolProvider, Some(2), false),
        ];
        let edges = vec![make_edge("tools-a", "tools-b")];
        let graph = make_graph(nodes, edges);
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(!report.valid);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "layer_violation"));
    }

    #[test]
    fn test_same_layer_violation_mutual() {
        // Mutual dependencies at the same layer
        let nodes = vec![
            make_node_old("layer2-a", NodeKind::ToolProvider, Some(2), false),
            make_node_old("layer2-b", NodeKind::PackageProvider, Some(2), false),
        ];
        let edges = vec![
            make_edge("layer2-a", "layer2-b"),
            make_edge("layer2-b", "layer2-a"),
        ];
        let graph = make_graph(nodes, edges);
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(!report.valid);
        let layer_violations: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "layer_violation")
            .collect();
        assert!(layer_violations.len() >= 2);
    }

    // -----------------------------------------------------------------------
    // URL mismatch tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_url_mismatch_detected() {
        // Node id "phenix-tools" but URL points to a different repo
        let nodes = vec![NodeSpec {
            id: "phenix-tools".to_string(),
            path: PathBuf::from("flakes/02-producers/phenix-tools"),
            repo_url: Some("https://github.com/other-org/wrong-repo.git".to_string()),
            kind: NodeKind::ToolProvider,
            role: RepoRole::Producer,
            layer: Some(2),
            is_root: false,
        }];
        let graph = make_graph(nodes, Vec::new());
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(!report.valid);
        assert!(report.diagnostics.iter().any(|d| d.code == "url_mismatch"));
    }

    #[test]
    fn test_url_mismatch_ssh_style() {
        // SSH-style URL with wrong repo name
        let nodes = vec![NodeSpec {
            id: "phenix-tools".to_string(),
            path: PathBuf::from("flakes/02-producers/phenix-tools"),
            repo_url: Some("git@github.com:other-org/wrong-repo.git".to_string()),
            kind: NodeKind::ToolProvider,
            role: RepoRole::Producer,
            layer: Some(2),
            is_root: false,
        }];
        let graph = make_graph(nodes, Vec::new());
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(!report.valid);
        assert!(report.diagnostics.iter().any(|d| d.code == "url_mismatch"));
    }

    #[test]
    fn test_url_mismatch_ok() {
        // URL matches node id
        let nodes = vec![NodeSpec {
            id: "phenix-tools".to_string(),
            path: PathBuf::from("flakes/02-producers/phenix-tools"),
            repo_url: Some("https://github.com/matthis-k/phenix-tools.git".to_string()),
            kind: NodeKind::ToolProvider,
            role: RepoRole::Producer,
            layer: Some(2),
            is_root: false,
        }];
        let graph = make_graph(nodes, Vec::new());
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(report.valid);
        assert!(!report.diagnostics.iter().any(|d| d.code == "url_mismatch"));
    }

    #[test]
    fn test_url_mismatch_ok_ssh() {
        // SSH-style URL matching
        let nodes = vec![NodeSpec {
            id: "phenix-tools".to_string(),
            path: PathBuf::from("flakes/02-producers/phenix-tools"),
            repo_url: Some("git@github.com:matthis-k/phenix-tools.git".to_string()),
            kind: NodeKind::ToolProvider,
            role: RepoRole::Producer,
            layer: Some(2),
            is_root: false,
        }];
        let graph = make_graph(nodes, Vec::new());
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(report.valid);
        assert!(!report.diagnostics.iter().any(|d| d.code == "url_mismatch"));
    }

    #[test]
    fn test_url_mismatch_ok_no_git_suffix() {
        // URL without .git suffix
        let nodes = vec![NodeSpec {
            id: "phenix-host".to_string(),
            path: PathBuf::from("flakes/05-consumers/phenix-host"),
            repo_url: Some("https://github.com/matthis-k/phenix-host".to_string()),
            kind: NodeKind::HostConsumer,
            role: RepoRole::Consumer,
            layer: Some(5),
            is_root: false,
        }];
        let graph = make_graph(nodes, Vec::new());
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(report.valid);
        assert!(!report.diagnostics.iter().any(|d| d.code == "url_mismatch"));
    }

    // -----------------------------------------------------------------------
    // Missing repo path tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_missing_repo_path_detected() {
        // Path does not exist on disk
        let nodes = vec![NodeSpec {
            id: "phenix-tools".to_string(),
            path: PathBuf::from("/tmp/__stitch_test_nonexistent_path_that_should_not_exist__"),
            repo_url: None,
            kind: NodeKind::ToolProvider,
            role: RepoRole::Producer,
            layer: Some(2),
            is_root: false,
        }];
        let graph = make_graph(nodes, Vec::new());
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(!report
            .diagnostics
            .iter()
            .any(|d| d.code.contains("path") && d.severity == DiagnosticSeverity::Error));
        // It should be a warning for missing path
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "missing_repo_path"));
    }

    #[test]
    fn test_missing_repo_path_skips_empty_path() {
        // Nodes with empty path (PathBuf::new()) must not trigger the check
        let nodes = vec![make_node_old("pins", NodeKind::Pins, Some(0), false)];
        let graph = make_graph(nodes, Vec::new());
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(!report
            .diagnostics
            .iter()
            .any(|d| d.code == "missing_repo_path"));
    }

    #[test]
    fn test_missing_repo_path_ok_with_tempdir() {
        // Path exists as a temp directory — must pass
        let dir = tempfile::tempdir().expect("tempdir creation");
        let path = dir.path().to_path_buf();
        let nodes = vec![NodeSpec {
            id: "existing-repo".to_string(),
            path,
            repo_url: None,
            kind: NodeKind::ToolProvider,
            role: RepoRole::Producer,
            layer: Some(2),
            is_root: false,
        }];
        let graph = make_graph(nodes, Vec::new());
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(!report
            .diagnostics
            .iter()
            .any(|d| d.code == "missing_repo_path"));
    }

    // -----------------------------------------------------------------------
    // URL extraction unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_url_repo_name_https() {
        assert_eq!(
            url_repo_name("https://github.com/matthis-k/phenix-tools.git"),
            Some("phenix-tools")
        );
    }

    #[test]
    fn test_url_repo_name_https_no_git_suffix() {
        assert_eq!(
            url_repo_name("https://github.com/matthis-k/phenix-host"),
            Some("phenix-host")
        );
    }

    #[test]
    fn test_url_repo_name_ssh() {
        assert_eq!(
            url_repo_name("git@github.com:matthis-k/phenix-tools.git"),
            Some("phenix-tools")
        );
    }

    #[test]
    fn test_url_repo_name_ssh_no_git_suffix() {
        assert_eq!(
            url_repo_name("git@github.com:matthis-k/phenix-host"),
            Some("phenix-host")
        );
    }

    #[test]
    fn test_url_repo_name_plain_path() {
        assert_eq!(url_repo_name("/some/path/to/repo-name"), Some("repo-name"));
    }

    // -----------------------------------------------------------------------
    // Valid workspace with path/layer consistency
    // -----------------------------------------------------------------------

    #[test]
    fn test_valid_workspace_full_path_layer_consistency() {
        // Full workspace where all paths and layers match
        type NodeSpecTuple<'a> = (
            &'a str,
            NodeKind,
            RepoRole,
            u32,
            &'a str,
            Option<&'a str>,
            bool,
        );
        let node_specs: Vec<NodeSpecTuple<'_>> = vec![
            (
                "phenix-pins",
                NodeKind::Pins,
                RepoRole::Pins,
                0,
                "flakes/00-pins/phenix-pins",
                Some("https://github.com/matthis-k/phenix-pins.git"),
                false,
            ),
            (
                "phenix-packages",
                NodeKind::PackageProvider,
                RepoRole::PkgsAggregator,
                4,
                "flakes/04-packages/phenix-packages",
                None,
                false,
            ),
            (
                "phenix-tools",
                NodeKind::ToolProvider,
                RepoRole::Producer,
                2,
                "flakes/02-producers/phenix-tools",
                Some("https://github.com/matthis-k/phenix-tools.git"),
                false,
            ),
            (
                "phenix-hosts",
                NodeKind::HostConsumer,
                RepoRole::Consumer,
                5,
                "flakes/05-consumers/phenix-hosts",
                None,
                false,
            ),
            (
                "phenix",
                NodeKind::WorkspaceRoot,
                RepoRole::Root,
                6,
                "",
                None,
                true,
            ),
        ];

        let mut nodes = Vec::new();
        for (id, kind, role, layer, path, url, is_root) in &node_specs {
            nodes.push(NodeSpec {
                id: id.to_string(),
                path: PathBuf::from(path),
                repo_url: url.map(|s| s.to_string()),
                kind: *kind,
                role: *role,
                layer: Some(*layer),
                is_root: *is_root,
            });
        }

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
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(
            report.valid,
            "Expected valid workspace, got diagnostics: {:#?}",
            report
                .diagnostics
                .iter()
                .map(|d| format!("[{}] {}", d.code, d.message))
                .collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // Existing rule edge-case coverage
    // -----------------------------------------------------------------------

    #[test]
    fn test_duplicate_edge_warning() {
        let nodes = vec![
            make_node_old("consumer", NodeKind::HostConsumer, Some(5), false),
            make_node_old("provider", NodeKind::Pins, Some(0), false),
        ];
        let edges = vec![
            make_edge("consumer", "provider"),
            make_edge("consumer", "provider"),
        ];
        let graph = make_graph(nodes, edges);
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        let dups: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "duplicate_edge")
            .collect();
        assert_eq!(dups.len(), 1);
    }

    #[test]
    fn test_unknown_source_node() {
        let nodes = vec![make_node_old("known", NodeKind::Pins, Some(0), false)];
        let edges = vec![make_edge("unknown", "known")];
        let graph = make_graph(nodes, edges);
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(!report.valid);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "unknown_source_node"));
    }

    #[test]
    fn test_unknown_target_node() {
        let nodes = vec![make_node_old("known", NodeKind::Pins, Some(0), false)];
        let edges = vec![make_edge("known", "unknown")];
        let graph = make_graph(nodes, edges);
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        assert!(!report.valid);
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.code == "unknown_target_node"));
    }

    #[test]
    fn test_strict_mode_missing_layer_info() {
        // In strict mode, missing layers on edges produce a Warning
        let nodes = vec![
            make_node_old("a", NodeKind::Unknown, None, false),
            make_node_old("b", NodeKind::Unknown, None, false),
        ];
        let edges = vec![make_edge("a", "b")];
        let graph = make_graph(nodes, edges);
        let report = validate_snapshot(&graph, &ValidateOptions { strict: true });
        let missing: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "missing_layer")
            .collect();
        assert!(!missing.is_empty());
        assert_eq!(missing[0].severity, DiagnosticSeverity::Warning);
    }

    #[test]
    fn test_non_strict_mode_missing_layer_info() {
        // In non-strict mode, missing layers produce Info
        let nodes = vec![
            make_node_old("a", NodeKind::Unknown, None, false),
            make_node_old("b", NodeKind::Unknown, None, false),
        ];
        let edges = vec![make_edge("a", "b")];
        let graph = make_graph(nodes, edges);
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        let missing: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "missing_layer")
            .collect();
        assert!(!missing.is_empty());
        assert_eq!(missing[0].severity, DiagnosticSeverity::Info);
    }

    #[test]
    fn test_external_input_multi_owner_warning() {
        let nodes = vec![
            make_node_old("repo-a", NodeKind::ToolProvider, Some(2), false),
            make_node_old("repo-b", NodeKind::HostConsumer, Some(5), false),
        ];
        let external_inputs = vec![
            crate::graph::ExternalInput {
                owner_node: "repo-a".into(),
                input_name: "nixpkgs".into(),
                locked_type: Some("github".into()),
                url_or_repo: None,
                rev: None,
            },
            crate::graph::ExternalInput {
                owner_node: "repo-b".into(),
                input_name: "nixpkgs".into(),
                locked_type: Some("github".into()),
                url_or_repo: None,
                rev: None,
            },
        ];
        let mut graph = make_graph(nodes, Vec::new());
        graph.external_inputs = external_inputs;
        let report = validate_snapshot(&graph, &ValidateOptions::default());
        let multi: Vec<_> = report
            .diagnostics
            .iter()
            .filter(|d| d.code == "external_input_multi_owner")
            .collect();
        assert_eq!(multi.len(), 1);
    }
}
