use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::graph;
use crate::model::WorkspaceConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    All,
    Changed,
    Dirty,
    Current,
    Explicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureMode {
    SelfOnly,
    Upstream,
    Downstream,
    Connected,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderMode {
    Stable,
    ProvidersFirst,
    ConsumersFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    ReadOnly,
    Mutating,
}

#[derive(Debug, Clone)]
pub struct ExecutionScope {
    pub selection: SelectionMode,
    pub explicit_nodes: Vec<String>,
    pub closure: ClosureMode,
    pub order: OrderMode,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionNode {
    pub name: String,
    pub path: PathBuf,
    pub role: String,
    pub layer: u32,
    pub directly_selected: bool,
    pub directly_changed: bool,
}

#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    pub nodes: Vec<ExecutionNode>,
    pub argv: Vec<String>,
    pub mode: ExecutionMode,
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub dry_run: bool,
    pub apply: bool,
    pub keep_going: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeResult {
    pub node: String,
    pub path: PathBuf,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionReport {
    pub node_results: Vec<NodeResult>,
    pub total_nodes: usize,
    pub successful_nodes: usize,
    pub failed_nodes: usize,
    pub stopped_early: bool,
}

pub fn parse_selection_mode(value: &str) -> Result<SelectionMode, String> {
    match value {
        "all" => Ok(SelectionMode::All),
        "changed" => Ok(SelectionMode::Changed),
        "dirty" => Ok(SelectionMode::Dirty),
        "current" => Ok(SelectionMode::Current),
        "explicit" => Ok(SelectionMode::Explicit),
        _ => Err(format!("unknown selection '{value}'")),
    }
}

pub fn parse_closure_mode(value: &str) -> Result<ClosureMode, String> {
    match value {
        "self" => Ok(ClosureMode::SelfOnly),
        "upstream" => Ok(ClosureMode::Upstream),
        "downstream" => Ok(ClosureMode::Downstream),
        "connected" => Ok(ClosureMode::Connected),
        "all" => Ok(ClosureMode::All),
        _ => Err(format!("unknown closure '{value}'")),
    }
}

pub fn parse_order_mode(value: &str) -> Result<OrderMode, String> {
    match value {
        "stable" => Ok(OrderMode::Stable),
        "providers-first" => Ok(OrderMode::ProvidersFirst),
        "consumers-first" => Ok(OrderMode::ConsumersFirst),
        _ => Err(format!("unknown order '{value}'")),
    }
}

pub fn parse_execution_mode(value: &str) -> Result<ExecutionMode, String> {
    match value {
        "read-only" | "readonly" => Ok(ExecutionMode::ReadOnly),
        "mutating" => Ok(ExecutionMode::Mutating),
        _ => Err(format!("unknown execution mode '{value}'")),
    }
}

pub fn build_scope(
    config: &WorkspaceConfig,
    scope: &ExecutionScope,
) -> Result<Vec<ExecutionNode>, String> {
    let graph = load_canonical_graph(config)?;
    let all_names = config
        .repos
        .iter()
        .map(|repo| repo.name.clone())
        .collect::<Vec<_>>();
    let selected = select_nodes(config, scope)?;
    let planner = graph::DagPlanner::new(&graph);
    let stable_order = all_names.clone();
    let plan = planner.plan(&graph::DagPlanRequest {
        selection: if scope.selection == SelectionMode::All {
            graph::PlanSelectionMode::All
        } else {
            graph::PlanSelectionMode::Explicit
        },
        explicit_nodes: selected.clone(),
        closure: match scope.closure {
            ClosureMode::SelfOnly => graph::PlanClosureMode::SelfOnly,
            ClosureMode::Upstream => graph::PlanClosureMode::Upstream,
            ClosureMode::Downstream => graph::PlanClosureMode::Downstream,
            ClosureMode::Connected => graph::PlanClosureMode::Connected,
            ClosureMode::All => graph::PlanClosureMode::All,
        },
        order: match scope.order {
            OrderMode::Stable => graph::PlanOrderMode::Stable,
            OrderMode::ProvidersFirst => graph::PlanOrderMode::ProvidersFirst,
            OrderMode::ConsumersFirst => graph::PlanOrderMode::ConsumersFirst,
        },
        stable_order,
    })?;
    let selected_set = selected.into_iter().collect::<BTreeSet<_>>();

    plan.nodes
        .into_iter()
        .map(|planned| {
            let repository = config
                .repos
                .iter()
                .find(|repo| repo.name == planned.name)
                .ok_or_else(|| format!("unknown planned node '{}'", planned.name))?;
            let node = graph
                .node(&planned.name)
                .ok_or_else(|| format!("graph node '{}' disappeared", planned.name))?;
            Ok(ExecutionNode {
                name: planned.name.clone(),
                path: repository.resolved_path(config),
                role: node.role.as_str().to_string(),
                layer: node.layer.or_else(|| node.role.layer()).unwrap_or(255),
                directly_selected: selected_set.contains(&planned.name),
                directly_changed: repository_changed(&repository.resolved_path(config)),
            })
        })
        .collect()
}

pub fn build_plan(
    config: &WorkspaceConfig,
    scope: &ExecutionScope,
    argv: Vec<String>,
    mode: ExecutionMode,
) -> Result<ExecutionPlan, String> {
    if argv.is_empty() {
        return Err("an argv vector is required after --".to_string());
    }
    Ok(ExecutionPlan {
        nodes: build_scope(config, scope)?,
        argv,
        mode,
    })
}

pub fn run_plan(plan: &ExecutionPlan, options: &RunOptions) -> Result<ExecutionReport, String> {
    if plan.mode == ExecutionMode::Mutating && !options.apply && !options.dry_run {
        return Err("mutating execution requires --apply or --dry-run".to_string());
    }
    let mut results = Vec::new();
    let total = plan.nodes.len();
    let mut stopped_early = false;

    for (index, node) in plan.nodes.iter().enumerate() {
        if options.dry_run {
            results.push(NodeResult {
                node: node.name.clone(),
                path: node.path.clone(),
                success: true,
                exit_code: Some(0),
                stdout: format!("would execute: {}", plan.argv.join(" ")),
                stderr: String::new(),
            });
            continue;
        }

        let output = Command::new(&plan.argv[0])
            .args(&plan.argv[1..])
            .current_dir(&node.path)
            .env("STITCH_NODE_ID", &node.name)
            .env("STITCH_NODE_PATH", &node.path)
            .env("STITCH_NODE_ROLE", &node.role)
            .env("STITCH_NODE_LAYER", node.layer.to_string())
            .env("STITCH_NODE_INDEX", index.to_string())
            .env("STITCH_NODE_TOTAL", total.to_string())
            .env(
                "STITCH_NODE_DIRECTLY_SELECTED",
                node.directly_selected.to_string(),
            )
            .env(
                "STITCH_NODE_DIRECTLY_CHANGED",
                node.directly_changed.to_string(),
            )
            .output()
            .map_err(|error| {
                format!(
                    "execute '{}' in {}: {error}",
                    plan.argv[0],
                    node.path.display()
                )
            })?;
        let result = NodeResult {
            node: node.name.clone(),
            path: node.path.clone(),
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        };
        let failed = !result.success;
        results.push(result);
        if failed && !options.keep_going {
            stopped_early = index + 1 < total;
            break;
        }
    }

    let successful = results.iter().filter(|result| result.success).count();
    let failed = results.len() - successful;
    Ok(ExecutionReport {
        node_results: results,
        total_nodes: total,
        successful_nodes: successful,
        failed_nodes: failed,
        stopped_early,
    })
}

fn select_nodes(config: &WorkspaceConfig, scope: &ExecutionScope) -> Result<Vec<String>, String> {
    match scope.selection {
        SelectionMode::All => Ok(config.repos.iter().map(|repo| repo.name.clone()).collect()),
        SelectionMode::Changed | SelectionMode::Dirty => Ok(config
            .repos
            .iter()
            .filter(|repo| repository_changed(&repo.resolved_path(config)))
            .map(|repo| repo.name.clone())
            .collect()),
        SelectionMode::Explicit => {
            if scope.explicit_nodes.is_empty() {
                return Err("explicit selection requires --node".to_string());
            }
            for name in &scope.explicit_nodes {
                if !config.repos.iter().any(|repo| &repo.name == name) {
                    return Err(format!("unknown node '{name}'"));
                }
            }
            Ok(scope.explicit_nodes.clone())
        }
        SelectionMode::Current => {
            let cwd = std::env::current_dir().map_err(|error| error.to_string())?;
            let canonical = cwd.canonicalize().unwrap_or(cwd);
            config
                .repos
                .iter()
                .find(|repo| {
                    canonical.starts_with(
                        repo.resolved_path(config)
                            .canonicalize()
                            .unwrap_or_else(|_| repo.resolved_path(config)),
                    )
                })
                .map(|repo| vec![repo.name.clone()])
                .ok_or_else(|| {
                    "current directory is not inside a discovered repository".to_string()
                })
        }
    }
}

fn repository_changed(path: &Path) -> bool {
    Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(path)
        .output()
        .map(|output| output.status.success() && !output.stdout.is_empty())
        .unwrap_or(false)
}

pub(crate) fn load_canonical_graph(
    config: &WorkspaceConfig,
) -> Result<graph::CanonicalWorkspaceGraph, String> {
    graph::derive_workspace_graph_from_config(config, None)
        .map_err(|error| format!("derive workspace graph: {error}"))
}
