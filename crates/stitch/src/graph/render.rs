use crate::graph::{validate::GraphValidationReport, CanonicalWorkspaceGraph, WorkspaceGraphDraft};
use crate::model::WorkspaceConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderFormat {
    Text,
    Json,
    Mermaid,
}

fn render_graph_snapshot(
    graph: &WorkspaceGraphDraft,
    format: RenderFormat,
) -> Result<String, String> {
    match format {
        RenderFormat::Json => render_graph_json(graph),
        RenderFormat::Text => render_graph_text(graph),
        RenderFormat::Mermaid => render_graph_mermaid(graph),
    }
}

pub fn render_graph_derive(
    graph: &CanonicalWorkspaceGraph,
    format: RenderFormat,
) -> Result<String, String> {
    render_graph_snapshot(&graph.to_snapshot(), format)
}

pub fn render_order(
    graph: &CanonicalWorkspaceGraph,
    order: &[String],
    format: RenderFormat,
) -> Result<String, String> {
    render_order_snapshot(&graph.to_snapshot(), order, format)
}

pub fn render_validation_report(
    report: &GraphValidationReport,
    format: RenderFormat,
) -> Result<String, String> {
    match format {
        RenderFormat::Json => render_validation_json(report),
        RenderFormat::Text => render_validation_text(report),
        RenderFormat::Mermaid => render_validation_mermaid(report),
    }
}

fn render_order_snapshot(
    graph: &WorkspaceGraphDraft,
    order: &[String],
    format: RenderFormat,
) -> Result<String, String> {
    match format {
        RenderFormat::Json => render_order_json(order),
        RenderFormat::Text => render_order_text(order, graph),
        RenderFormat::Mermaid => render_order_mermaid(graph, order),
    }
}

pub fn operation_dag_json(
    cfg: &WorkspaceConfig,
    mode: &str,
    split: &str,
    staged: bool,
    verification: Option<(&str, &str)>,
    repo_filter: &[String],
) -> Result<serde_json::Value, String> {
    let statuses = crate::status::collect_all(cfg)?;
    let repo_filter: std::collections::BTreeSet<&String> = repo_filter.iter().collect();
    let mut nodes: Vec<serde_json::Value> = Vec::new();
    let mut evidence_message: Option<&str> = None;

    for s in &statuses {
        if !repo_filter.is_empty() && !repo_filter.contains(&s.name) {
            continue;
        }
        if !s.is_dirty {
            continue;
        }
        let repo_cfg = cfg.repos.iter().find(|r| r.name == s.name);
        let diff = repo_cfg
            .map(|r| {
                let p = r.resolved_path(cfg);
                if staged {
                    crate::git::git_diff_cached_names(&p).unwrap_or_default()
                } else {
                    crate::git::git_diff_names(&p).unwrap_or_default()
                }
            })
            .unwrap_or_default();

        if let Some((profile, context)) = verification.filter(|_| mode != "sync") {
            nodes.push(serde_json::json!({
                "id": format!("{}:precheck", s.name),
                "kind": "check",
                "repo": s.name,
                "command": [
                    "tend",
                    "check",
                    "--profile",
                    profile,
                    "--context",
                    context
                ],
                "depends_on": []
            }));
        }

        match split {
            "by-path" => {
                let mut by_dir: std::collections::BTreeMap<String, Vec<String>> =
                    std::collections::BTreeMap::new();
                for f in &diff {
                    let dir = f
                        .rfind('/')
                        .map(|i| f[..i].to_string())
                        .unwrap_or_else(|| "root".to_string());
                    by_dir.entry(dir).or_default().push(f.clone());
                }
                for (dir, files) in &by_dir {
                    let deps = if verification.is_some() && mode != "sync" {
                        vec![format!("{}:precheck", s.name)]
                    } else {
                        vec![]
                    };
                    nodes.push(serde_json::json!({
                        "id": format!("{}:{}", s.name, dir.replace('/', "_")),
                        "kind": "commit", "repo": s.name, "files": files,
                        "depends_on": deps
                    }));
                }
            }
            _ => {
                let deps = if verification.is_some() && mode != "sync" {
                    vec![format!("{}:precheck", s.name)]
                } else {
                    vec![]
                };
                nodes.push(serde_json::json!({
                    "id": format!("{}:commit", s.name),
                    "kind": "commit", "repo": s.name, "files": diff,
                    "depends_on": deps
                }));
            }
        }
    }

    if mode == "full" || mode == "sync" {
        let commit_ids: Vec<String> = nodes
            .iter()
            .filter(|n| n["kind"] == "commit")
            .filter_map(|n| n["id"].as_str().map(str::to_string))
            .collect();
        if !commit_ids.is_empty() {
            let workspace_root = cfg
                .config_dir
                .as_deref()
                .unwrap_or(std::path::Path::new("."));
            if let Some(root) = cfg.repos.iter().find(|repo| {
                repo.resolved_path(cfg)
                    .canonicalize()
                    .unwrap_or_else(|_| repo.resolved_path(cfg))
                    == workspace_root
                        .canonicalize()
                        .unwrap_or_else(|_| workspace_root.to_path_buf())
            }) {
                nodes.push(serde_json::json!({
                    "id": format!("{}:update-pins", root.name),
                    "kind": "update-pins", "repo": root.name,
                    "files": ["flake.lock"],
                    "depends_on": commit_ids
                }));
            }
        }
    }

    if mode == "full" && nodes.is_empty() {
        let root = cfg
            .config_dir
            .as_deref()
            .unwrap_or(std::path::Path::new("."));
        let metadata = root.join(".stitch").join("topology.json");
        let metadata = metadata.exists().then_some(metadata);
        let graph = crate::graph::derive::derive_workspace_graph(root, metadata.as_deref())
            .map_err(|e| format!("Workspace graph evidence failed: {e}"))?;
        let canonical = graph;
        let stable_order = cfg.repos.iter().map(|r| r.name.clone()).collect();
        let plan =
            crate::graph::DagPlanner::new(&canonical).plan(&crate::graph::DagPlanRequest {
                selection: crate::graph::PlanSelectionMode::All,
                explicit_nodes: Vec::new(),
                closure: crate::graph::PlanClosureMode::All,
                order: crate::graph::PlanOrderMode::ProvidersFirst,
                stable_order,
            })?;
        for planned in plan.nodes {
            if let Some(node) = canonical.node(&planned.name) {
                nodes.push(serde_json::json!({
                    "id": format!("{}:workspace", planned.name),
                    "kind": "workspace",
                    "repo": planned.name,
                    "path": node.path,
                    "layer": node.layer.unwrap_or(999),
                    "role": format!("{:?}", node.role),
                    "depends_on": []
                }));
            }
        }
        evidence_message = Some("No dirty repos; showing validated discovered-workspace graph");
    }

    let total = nodes.len();
    Ok(
        serde_json::json!({ "nodes": nodes, "total": total, "mode": mode, "message": evidence_message }),
    )
}

fn render_graph_text(graph: &WorkspaceGraphDraft) -> Result<String, String> {
    let mut out = String::new();

    out.push_str("Workspace DAG:\n\n");
    out.push_str("Nodes:\n");
    for node in graph.nodes.values() {
        let layer = node
            .layer
            .map(|l| format!("layer={l}"))
            .unwrap_or_else(|| "no layer".to_string());
        let kind = node_kind_name(&node.kind);
        let root = if node.is_root { " [ROOT]" } else { "" };
        out.push_str(&format!(
            "  {:<20} {:<12} kind={}{root}\n",
            node.id, layer, kind
        ));
    }

    if graph.edges.is_empty() {
        out.push_str("\nEdges: (none)\n");
    } else {
        out.push_str("\nEdges:\n");
        for edge in &graph.edges {
            let reason = match &edge.kind {
                super::EdgeKind::FlakeInput { input_name, .. } => {
                    format!("input={input_name}")
                }
                super::EdgeKind::Manual { .. } => "manual".to_string(),
                super::EdgeKind::SubmoduleMembership { .. } => "submodule-membership".to_string(),
            };
            out.push_str(&format!(
                "  {:<20} -> {:<20}  {reason}\n",
                edge.from, edge.to
            ));
        }
    }

    if !graph.external_inputs.is_empty() {
        out.push_str("\nExternal inputs:\n");
        for ext in &graph.external_inputs {
            out.push_str(&format!(
                "  {:<20} input={:<20} type={:?} url={:?}\n",
                ext.owner_node, ext.input_name, ext.locked_type, ext.url_or_repo
            ));
        }
    }

    Ok(out)
}

fn render_graph_json(graph: &WorkspaceGraphDraft) -> Result<String, String> {
    serde_json::to_string_pretty(graph).map_err(|e| format!("JSON serialization: {e}"))
}

fn render_graph_mermaid(graph: &WorkspaceGraphDraft) -> Result<String, String> {
    let mut out = String::new();
    out.push_str("flowchart TD\n");

    for node in graph.nodes.values() {
        let label = mermaid_label(node);
        out.push_str(&format!("  {}[{}]\n", mermaid_id(&node.id), label));
    }

    for edge in &graph.edges {
        out.push_str(&format!(
            "  {} --> {}\n",
            mermaid_id(&edge.from),
            mermaid_id(&edge.to)
        ));
    }

    Ok(out)
}

fn render_validation_text(report: &GraphValidationReport) -> Result<String, String> {
    let mut out = String::new();

    if report.valid {
        out.push_str("Workspace DAG: VALID\n");
    } else {
        out.push_str("Workspace DAG: INVALID\n");
    }

    out.push_str(&format!(
        "  Nodes: {}  Edges: {}\n\n",
        report.node_count, report.edge_count
    ));

    if report.diagnostics.is_empty() {
        out.push_str("No diagnostics.\n");
    } else {
        out.push_str("Diagnostics:\n");
        for diag in &report.diagnostics {
            let sev = match diag.severity {
                super::validate::DiagnosticSeverity::Error => "ERROR",
                super::validate::DiagnosticSeverity::Warning => "WARN",
                super::validate::DiagnosticSeverity::Info => "INFO",
            };
            out.push_str(&format!("  [{sev:5}] {}: {}\n", diag.code, diag.message));
        }
    }

    Ok(out)
}

fn render_validation_json(report: &GraphValidationReport) -> Result<String, String> {
    serde_json::to_string_pretty(report).map_err(|e| format!("JSON serialization: {e}"))
}

fn render_validation_mermaid(report: &GraphValidationReport) -> Result<String, String> {
    let mut out = String::new();
    if report.valid {
        out.push_str("```\nWorkspace DAG: VALID\n```\n");
    } else {
        out.push_str("```\nWorkspace DAG: INVALID\n");
        for diag in &report.diagnostics {
            out.push_str(&format!("  {}: {}\n", diag.code, diag.message));
        }
        out.push_str("```\n");
    }
    Ok(out)
}

fn render_order_text(order: &[String], graph: &WorkspaceGraphDraft) -> Result<String, String> {
    let mut out = String::new();
    out.push_str("Provider-before-consumer order:\n");
    for (i, id) in order.iter().enumerate() {
        let layer = graph
            .nodes
            .get(id)
            .and_then(|n| n.layer)
            .map(|l| format!("layer={l}"))
            .unwrap_or_default();
        out.push_str(&format!("  {}. {:<20} {layer}\n", i + 1, id));
    }
    Ok(out)
}

fn render_order_json(order: &[String]) -> Result<String, String> {
    serde_json::to_string_pretty(&serde_json::json!({ "order": order }))
        .map_err(|e| format!("JSON serialization: {e}"))
}

fn render_order_mermaid(graph: &WorkspaceGraphDraft, order: &[String]) -> Result<String, String> {
    let mut out = String::new();
    out.push_str("flowchart LR\n");
    out.push_str("  subgraph Order[Provider-before-consumer order]\n");
    out.push_str("    direction LR\n");
    for (i, id) in order.iter().enumerate() {
        let label = mermaid_label(graph.nodes.get(id).unwrap());
        out.push_str(&format!("    {}[{}]\n", mermaid_id(id), label));
        if i < order.len() - 1 {
            out.push_str(&format!(
                "    {} --> {}\n",
                mermaid_id(id),
                mermaid_id(&order[i + 1])
            ));
        }
    }
    out.push_str("  end\n");
    Ok(out)
}

fn mermaid_id(id: &str) -> String {
    id.replace(['-', '.'], "_")
}

fn mermaid_label(node: &super::NodeSpec) -> String {
    let layer = node
        .layer
        .map(|l| format!("layer {l}"))
        .unwrap_or_else(|| "no layer".to_string());
    format!(
        "{}<br/>{}<br/>{}",
        node.id,
        layer,
        node_kind_name(&node.kind)
    )
}

fn node_kind_name(kind: &super::NodeKind) -> &'static str {
    match kind {
        super::NodeKind::Pins => "pins",
        super::NodeKind::PackageProvider => "packageProvider",
        super::NodeKind::ToolProvider => "toolProvider",
        super::NodeKind::ShellProvider => "shellProvider",
        super::NodeKind::DesktopProvider => "desktopProvider",
        super::NodeKind::HostConsumer => "hostConsumer",
        super::NodeKind::WorkspaceRoot => "workspaceRoot",
        super::NodeKind::External => "external",
        super::NodeKind::Unknown => "unknown",
    }
}
