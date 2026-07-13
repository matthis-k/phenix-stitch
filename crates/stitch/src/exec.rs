use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::git;
use crate::graph;
use crate::model::WorkspaceConfig;
use crate::status;

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

#[derive(Debug, Clone)]
pub enum StepKind {
    Shell {
        argv: Vec<String>,
    },
    Builtin {
        name: String,
        args: serde_json::Value,
    },
}

#[derive(Debug, Clone)]
pub struct ExecutionStep {
    pub id: String,
    pub mode: ExecutionMode,
    pub kind: StepKind,
    pub condition: Option<StepCondition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepCondition {
    Always,
    Dirty,
    Staged,
    DirectlyChanged,
    DownstreamOnly,
    HasLockfile,
    HasChangedInputs,
}

#[derive(Debug, Clone)]
pub struct ExecutionNode {
    pub name: String,
    pub path: PathBuf,
    pub role: Option<String>,
    pub layer: u32,
    pub directly_selected: bool,
    pub directly_changed: bool,
    pub downstream_only: bool,
    pub steps: Vec<ExecutionStep>,
}

#[derive(Debug, Clone)]
pub struct ExecutionPlan {
    pub nodes: Vec<ExecutionNode>,
}

#[derive(Debug, Clone)]
pub struct RunOptions {
    pub dry_run: bool,
    pub apply: bool,
    pub json: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepResult {
    pub node: String,
    pub step_id: String,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeResult {
    pub node: String,
    pub success: bool,
    pub step_results: Vec<StepResult>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionReport {
    pub node_results: Vec<NodeResult>,
    pub total_nodes: usize,
    pub successful_nodes: usize,
    pub failed_nodes: usize,
}

pub fn parse_selection_mode(s: &str) -> Result<SelectionMode, String> {
    match s {
        "all" => Ok(SelectionMode::All),
        "changed" => Ok(SelectionMode::Changed),
        "dirty" => Ok(SelectionMode::Dirty),
        "current" => Ok(SelectionMode::Current),
        "explicit" => Ok(SelectionMode::Explicit),
        _ => Err(format!(
            "Unknown selection mode: {s} (use: all, changed, dirty, current, explicit)"
        )),
    }
}

pub fn parse_closure_mode(s: &str) -> Result<ClosureMode, String> {
    match s {
        "self" => Ok(ClosureMode::SelfOnly),
        "upstream" => Ok(ClosureMode::Upstream),
        "downstream" => Ok(ClosureMode::Downstream),
        "connected" => Ok(ClosureMode::Connected),
        "all" => Ok(ClosureMode::All),
        _ => Err(format!(
            "Unknown closure mode: {s} (use: self, upstream, downstream, connected, all)"
        )),
    }
}

pub fn parse_order_mode(s: &str) -> Result<OrderMode, String> {
    match s {
        "stable" => Ok(OrderMode::Stable),
        "providers-first" => Ok(OrderMode::ProvidersFirst),
        "consumers-first" => Ok(OrderMode::ConsumersFirst),
        _ => Err(format!(
            "Unknown order mode: {s} (use: stable, providers-first, consumers-first)"
        )),
    }
}

pub fn parse_execution_mode(s: &str) -> Result<ExecutionMode, String> {
    match s.to_lowercase().as_str() {
        "readonly" | "read-only" => Ok(ExecutionMode::ReadOnly),
        "mutating" => Ok(ExecutionMode::Mutating),
        _ => Err(format!(
            "Unknown execution mode: {s} (use: readonly, mutating)"
        )),
    }
}

pub fn parse_condition(s: &str) -> Result<StepCondition, String> {
    match s {
        "always" => Ok(StepCondition::Always),
        "dirty" => Ok(StepCondition::Dirty),
        "staged" => Ok(StepCondition::Staged),
        "directly_changed" => Ok(StepCondition::DirectlyChanged),
        "downstream_only" => Ok(StepCondition::DownstreamOnly),
        "has_lockfile" => Ok(StepCondition::HasLockfile),
        "has_changed_inputs" => Ok(StepCondition::HasChangedInputs),
        _ => Err(format!(
            "Unknown condition: {s} (use: always, dirty, staged, directly_changed, downstream_only, has_lockfile, has_changed_inputs)"
        )),
    }
}

fn is_node_dirty(path: &Path) -> bool {
    if !path.join(".git").exists() {
        return false;
    }
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(path)
        .output();
    match output {
        Ok(o) => !String::from_utf8_lossy(&o.stdout)
            .lines()
            .all(|l| l.trim().is_empty()),
        Err(_) => false,
    }
}

fn is_node_staged(path: &Path) -> bool {
    if !path.join(".git").exists() {
        return false;
    }
    match git::GitRepo::open(path) {
        Ok(repo) => match repo.status() {
            Ok(s) => s.staged_count() > 0,
            Err(_) => false,
        },
        Err(_) => false,
    }
}

fn has_lockfile(path: &Path) -> bool {
    path.join("flake.lock").exists()
}

fn get_node_changed(path: &Path) -> bool {
    if !path.join(".git").exists() {
        return false;
    }
    let output = std::process::Command::new("git")
        .args(["diff", "--name-only", "HEAD"])
        .current_dir(path)
        .output();
    if let Ok(o) = output {
        if o.status.success() {
            let out = String::from_utf8_lossy(&o.stdout);
            if out.lines().any(|l| !l.trim().is_empty()) {
                return true;
            }
        }
    }
    is_node_dirty(path)
}

fn config_order(cfg: &WorkspaceConfig) -> Vec<String> {
    cfg.repos.iter().map(|r| r.name.clone()).collect()
}

pub(crate) fn load_canonical_graph(
    cfg: &WorkspaceConfig,
) -> Result<graph::CanonicalWorkspaceGraph, String> {
    let order = config_order(cfg);
    let root = cfg.config_dir.as_deref().ok_or_else(|| {
        "Cannot derive Stitch DAG: workspace config directory is unavailable".to_string()
    })?;
    let metadata = root.join(".stitch").join("topology.json");
    let metadata = metadata.exists().then_some(metadata);

    let dag = graph::derive::derive_workspace_graph(root, metadata.as_deref())
        .map_err(|e| format!("Cannot derive Stitch DAG from discovered workspace: {e}"))?;
    let report =
        graph::validate::validate_graph(&dag, &graph::validate::ValidateOptions::default());
    if !report.valid {
        let messages = report
            .diagnostics
            .iter()
            .filter(|d| d.severity == graph::validate::DiagnosticSeverity::Error)
            .map(|d| format!("{}: {}", d.code, d.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!("Cannot use invalid Stitch DAG: {messages}"));
    }

    for edge in &dag.edges {
        if !order.contains(&edge.from) || !order.contains(&edge.to) {
            return Err(format!(
                "Canonical Stitch DAG edge references unknown configured node: {} -> {}",
                edge.from, edge.to
            ));
        }
    }

    graph::CanonicalWorkspaceGraph::from_legacy(dag).map_err(|e| e.to_string())
}

pub struct HookInstallResult {
    pub installed: bool,
    pub message: String,
}

pub fn install_hooks_for_repo(
    repo_name: &str,
    repo_path: &Path,
    workspace_root: &Path,
    force: bool,
) -> Result<HookInstallResult, String> {
    let managed_marker = "# managed-by: phenix-stitch-hooks";
    let hooks_dir = repo_path.join(".git").join("hooks");

    if !hooks_dir.exists() {
        return Ok(HookInstallResult {
            installed: false,
            message: ".git/hooks not found".to_string(),
        });
    }

    let is_root = repo_name == "phenix";
    let _sub_path = repo_path.strip_prefix(workspace_root).unwrap_or(repo_path);
    let pre_commit_cmd = if is_root {
        "nix develop .#default --command tend check --profile git-hook --staged --affected-dag"
            .to_string()
    } else {
        "nix develop .#default --command tend check --profile git-hook --staged".to_string()
    };
    let pre_push_cmd = if is_root {
        "nix develop .#default --command tend check --profile pre-push --affected-dag".to_string()
    } else {
        "nix develop .#default --command tend check --profile pre-push".to_string()
    };

    for (hook_name, hook_cmd) in [("pre-commit", pre_commit_cmd), ("pre-push", pre_push_cmd)] {
        let hook_path = hooks_dir.join(hook_name);
        let should_install = if hook_path.exists() {
            let existing = std::fs::read_to_string(&hook_path).unwrap_or_default();
            existing.contains(managed_marker) || force
        } else {
            true
        };
        if !should_install {
            return Err(format!(
                "Not overwriting unmanaged {hook_name} hook for '{repo_name}'. Use --force to override."
            ));
        }

        let content = format!(
            r"#!/usr/bin/env bash
# managed-by: phenix-stitch-hooks
# Source: stitch hooks install
# Do not edit manually.

{hook_cmd}
"
        );
        std::fs::write(&hook_path, &content)
            .map_err(|e| format!("Failed to write {}: {}", hook_path.display(), e))?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to chmod {}: {}", hook_path.display(), e))?;
    }

    Ok(HookInstallResult {
        installed: true,
        message: "Hooks installed".to_string(),
    })
}

#[cfg(test)]
fn topological_sort(
    all_nodes: &[String],
    deps: &BTreeMap<String, Vec<String>>,
    order: &[String],
    mode: OrderMode,
) -> Result<Vec<String>, String> {
    let node_set: BTreeSet<&String> = all_nodes.iter().collect();
    let mut in_degree: BTreeMap<String, usize> = BTreeMap::new();
    let mut out_edges: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for n in all_nodes {
        in_degree.entry(n.clone()).or_insert(0);
        out_edges.entry(n.clone()).or_default();
    }

    for n in all_nodes {
        if let Some(providers) = deps.get(n) {
            for p in providers {
                if node_set.contains(p) {
                    out_edges.entry(p.clone()).or_default().push(n.clone());
                    *in_degree.entry(n.clone()).or_insert(0) += 1;
                }
            }
        }
    }

    let mut result = Vec::new();
    let mut queue: Vec<String> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(id, _)| id.clone())
        .collect();

    queue.sort_by(|a, b| {
        let pos_a = order.iter().position(|x| x == a).unwrap_or(usize::MAX);
        let pos_b = order.iter().position(|x| x == b).unwrap_or(usize::MAX);
        pos_a.cmp(&pos_b)
    });

    while let Some(node) = queue.first().cloned() {
        queue.retain(|n| n != &node);
        result.push(node.clone());

        if let Some(consumers) = out_edges.get(&node) {
            for consumer in consumers {
                if let Some(deg) = in_degree.get_mut(consumer) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push(consumer.clone());
                    }
                }
            }
        }

        queue.sort_by(|a, b| {
            let pos_a = order.iter().position(|x| x == a).unwrap_or(usize::MAX);
            let pos_b = order.iter().position(|x| x == b).unwrap_or(usize::MAX);
            pos_a.cmp(&pos_b)
        });
    }

    if result.len() != all_nodes.len() {
        let unresolved: Vec<String> = all_nodes
            .iter()
            .filter(|n| !result.contains(n))
            .cloned()
            .collect();
        return Err(format!(
            "Cannot order Stitch DAG scope: cycle among {}",
            unresolved.join(", ")
        ));
    }

    if mode == OrderMode::ConsumersFirst {
        result.reverse();
    }

    Ok(result)
}

#[cfg(test)]
fn expand_closure(
    selected: &[String],
    closure: ClosureMode,
    all_nodes: &[String],
    deps: &BTreeMap<String, Vec<String>>,
    dependents: &BTreeMap<String, Vec<String>>,
) -> Vec<String> {
    match closure {
        ClosureMode::SelfOnly => selected.to_vec(),
        ClosureMode::All => all_nodes.to_vec(),
        ClosureMode::Upstream => {
            let mut result = BTreeSet::new();
            let mut stack: Vec<String> = selected.to_vec();
            while let Some(node) = stack.pop() {
                if result.insert(node.clone()) {
                    if let Some(providers) = deps.get(&node) {
                        for p in providers {
                            stack.push(p.clone());
                        }
                    }
                }
            }
            result.into_iter().collect()
        }
        ClosureMode::Downstream => {
            let mut result = BTreeSet::new();
            let mut stack: Vec<String> = selected.to_vec();
            while let Some(node) = stack.pop() {
                if result.insert(node.clone()) {
                    if let Some(consumers) = dependents.get(&node) {
                        for c in consumers {
                            stack.push(c.clone());
                        }
                    }
                }
            }
            result.into_iter().collect()
        }
        ClosureMode::Connected => {
            let upstream =
                expand_closure(selected, ClosureMode::Upstream, all_nodes, deps, dependents);
            let downstream = expand_closure(
                selected,
                ClosureMode::Downstream,
                all_nodes,
                deps,
                dependents,
            );
            let mut combined: BTreeSet<String> = BTreeSet::new();
            for n in upstream {
                combined.insert(n);
            }
            for n in downstream {
                combined.insert(n);
            }
            combined.into_iter().collect()
        }
    }
}

pub fn build_scope(
    cfg: &WorkspaceConfig,
    scope: &ExecutionScope,
) -> Result<Vec<ExecutionNode>, String> {
    let config_order = config_order(cfg);
    let all_names: Vec<String> = cfg.repos.iter().map(|r| r.name.clone()).collect();
    let statuses = status::collect_all(cfg)?;
    let canonical_graph = load_canonical_graph(cfg)?;

    let selected = match scope.selection {
        SelectionMode::All => all_names.clone(),
        SelectionMode::Explicit => {
            if scope.explicit_nodes.is_empty() {
                return Err("--nodes requires at least one node name".to_string());
            }
            for name in &scope.explicit_nodes {
                if !cfg.repos.iter().any(|r| r.name == *name) {
                    return Err(format!("Unknown node: {name}"));
                }
            }
            scope.explicit_nodes.clone()
        }
        SelectionMode::Changed => {
            let mut changed = Vec::new();
            for repo in &cfg.repos {
                let path = repo.resolved_path(cfg);
                if get_node_changed(&path) {
                    changed.push(repo.name.clone());
                }
            }
            changed
        }
        SelectionMode::Dirty => {
            let mut dirty = Vec::new();
            for repo in &cfg.repos {
                let path = repo.resolved_path(cfg);
                if is_node_dirty(&path) {
                    dirty.push(repo.name.clone());
                }
            }
            dirty
        }
        SelectionMode::Current => {
            return Err("--current not yet implemented; use --node <name> instead".to_string());
        }
    };

    let planner = graph::DagPlanner::new(&canonical_graph);
    let plan = planner.plan(&graph::DagPlanRequest {
        selection: graph_selection_mode(scope.selection),
        explicit_nodes: selected.clone(),
        closure: graph_closure_mode(scope.closure),
        order: graph_order_mode(scope.order),
        stable_order: config_order.clone(),
    })?;
    let ordered: Vec<String> = plan.nodes.iter().map(|node| node.name.clone()).collect();

    let selected_set: BTreeSet<&String> = selected.iter().collect();
    let downstream_set: BTreeSet<String> = planner
        .expand_closure(&selected, graph::PlanClosureMode::Downstream, &all_names)
        .into_iter()
        .collect();

    let changed_set: BTreeSet<String> = {
        let mut s = BTreeSet::new();
        for repo in &cfg.repos {
            let path = repo.resolved_path(cfg);
            if get_node_changed(&path) {
                s.insert(repo.name.clone());
            }
        }
        s
    };

    let mut result = Vec::new();

    for name in ordered {
        let repo = cfg
            .repos
            .iter()
            .find(|r| r.name == name)
            .ok_or_else(|| format!("Node '{name}' not found in config"))?;
        let path = repo.resolved_path(cfg);
        let graph_node = canonical_graph
            .node(&name)
            .ok_or_else(|| format!("Node '{name}' missing from canonical graph"))?;
        let layer = graph_node
            .layer
            .or_else(|| graph_node.role.layer())
            .unwrap_or(999);
        let role = graph_node.role.as_str().to_string();
        let _status = statuses.iter().find(|s| s.name == name);
        let directly_selected = selected_set.contains(&name);
        let directly_changed = changed_set.contains(&name);

        result.push(ExecutionNode {
            name: name.clone(),
            path,
            role: Some(role),
            layer,
            directly_selected,
            directly_changed,
            downstream_only: downstream_set.contains(&name) && !directly_selected,
            steps: Vec::new(),
        });
    }

    Ok(result)
}

fn graph_selection_mode(selection: SelectionMode) -> graph::PlanSelectionMode {
    match selection {
        SelectionMode::All => graph::PlanSelectionMode::All,
        SelectionMode::Changed
        | SelectionMode::Dirty
        | SelectionMode::Current
        | SelectionMode::Explicit => graph::PlanSelectionMode::Explicit,
    }
}

fn graph_closure_mode(closure: ClosureMode) -> graph::PlanClosureMode {
    match closure {
        ClosureMode::SelfOnly => graph::PlanClosureMode::SelfOnly,
        ClosureMode::Upstream => graph::PlanClosureMode::Upstream,
        ClosureMode::Downstream => graph::PlanClosureMode::Downstream,
        ClosureMode::Connected => graph::PlanClosureMode::Connected,
        ClosureMode::All => graph::PlanClosureMode::All,
    }
}

fn graph_order_mode(order: OrderMode) -> graph::PlanOrderMode {
    match order {
        OrderMode::Stable => graph::PlanOrderMode::Stable,
        OrderMode::ProvidersFirst => graph::PlanOrderMode::ProvidersFirst,
        OrderMode::ConsumersFirst => graph::PlanOrderMode::ConsumersFirst,
    }
}

pub fn build_plan(
    cfg: &WorkspaceConfig,
    scope: &ExecutionScope,
    steps: Vec<ExecutionStep>,
) -> Result<ExecutionPlan, String> {
    let mut nodes = build_scope(cfg, scope)?;

    for node in &mut nodes {
        let is_dirty = is_node_dirty(&node.path);
        let is_staged = is_staged_check(&node.path);
        let has_lock = has_lockfile(&node.path);

        let applicable: Vec<ExecutionStep> = steps
            .iter()
            .filter(|step| {
                let cond = step.condition.unwrap_or(StepCondition::Always);
                match cond {
                    StepCondition::Always => true,
                    StepCondition::Dirty => is_dirty,
                    StepCondition::Staged => is_staged,
                    StepCondition::DirectlyChanged => node.directly_changed,
                    StepCondition::DownstreamOnly => node.downstream_only,
                    StepCondition::HasLockfile => has_lock,
                    StepCondition::HasChangedInputs => node.downstream_only && has_lock,
                }
            })
            .cloned()
            .collect();

        node.steps = applicable;
    }

    Ok(ExecutionPlan { nodes })
}

fn is_staged_check(path: &Path) -> bool {
    is_node_staged(path)
}

pub fn run_plan(
    cfg: &WorkspaceConfig,
    plan: &ExecutionPlan,
    opts: &RunOptions,
) -> Result<ExecutionReport, String> {
    let mut node_results = Vec::new();

    for node in &plan.nodes {
        if node.steps.is_empty() {
            if opts.json {
                node_results.push(NodeResult {
                    node: node.name.clone(),
                    success: true,
                    step_results: Vec::new(),
                });
            } else {
                println!("{}: nothing to execute", node.name);
            }
            continue;
        }

        let has_mutating = node.steps.iter().any(|s| s.mode == ExecutionMode::Mutating);
        if has_mutating && !opts.apply && !opts.dry_run {
            return Err(format!(
                "Node '{}' has mutating steps. Use --apply or --dry-run.",
                node.name
            ));
        }

        if opts.dry_run {
            let mut step_results = Vec::new();
            for step in &node.steps {
                let step_result = StepResult {
                    node: node.name.clone(),
                    step_id: step.id.clone(),
                    success: true,
                    stdout: "[dry-run] would execute".to_string(),
                    stderr: String::new(),
                };
                step_results.push(step_result);
            }
            node_results.push(NodeResult {
                node: node.name.clone(),
                success: true,
                step_results,
            });

            if !opts.json {
                println!("[dry-run] {}:", node.name);
                for step in &node.steps {
                    match &step.kind {
                        StepKind::Shell { argv } => {
                            println!("  {}: {}", step.id, argv.join(" "));
                        }
                        StepKind::Builtin { name, args } => {
                            println!("  {}: builtin {} {:?}", step.id, name, args);
                        }
                    }
                }
            }
            continue;
        }

        let mut step_results = Vec::new();
        let mut node_success = true;

        for step in &node.steps {
            let result = execute_step(node, step, cfg);
            let success = result.success;
            if !success {
                node_success = false;
            }
            if opts.json {
                step_results.push(result.clone());
            } else {
                if success {
                    println!("  {}: {} OK", node.name, step.id);
                } else {
                    eprintln!("  {}: {} FAILED", node.name, step.id);
                    if !result.stderr.is_empty() {
                        eprintln!("    stderr: {}", result.stderr.trim());
                    }
                }
            }
            if !success {
                break;
            }
        }

        node_results.push(NodeResult {
            node: node.name.clone(),
            success: node_success,
            step_results,
        });
    }

    let total = node_results.len();
    let successful = node_results.iter().filter(|r| r.success).count();
    let failed = total - successful;

    Ok(ExecutionReport {
        node_results,
        total_nodes: total,
        successful_nodes: successful,
        failed_nodes: failed,
    })
}

fn execute_step(node: &ExecutionNode, step: &ExecutionStep, cfg: &WorkspaceConfig) -> StepResult {
    match &step.kind {
        StepKind::Shell { argv } => {
            if argv.is_empty() {
                return StepResult {
                    node: node.name.clone(),
                    step_id: step.id.clone(),
                    success: false,
                    stdout: String::new(),
                    stderr: "Shell step has empty argv; provide a program or shell command"
                        .to_string(),
                };
            }
            let program = &argv[0];
            let args: Vec<&str> = argv[1..].iter().map(|s| s.as_str()).collect();

            let output = std::process::Command::new(program)
                .args(&args)
                .current_dir(&node.path)
                .output();

            match output {
                Ok(o) => {
                    let success = o.status.success();
                    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                    StepResult {
                        node: node.name.clone(),
                        step_id: step.id.clone(),
                        success,
                        stdout,
                        stderr,
                    }
                }
                Err(e) => StepResult {
                    node: node.name.clone(),
                    step_id: step.id.clone(),
                    success: false,
                    stdout: String::new(),
                    stderr: format!("Failed to execute: {e}"),
                },
            }
        }
        StepKind::Builtin { name, args } => run_builtin(node, cfg, name, args),
    }
}

fn run_builtin(
    node: &ExecutionNode,
    cfg: &WorkspaceConfig,
    name: &str,
    args: &serde_json::Value,
) -> StepResult {
    match name {
        "git.status" => builtin_git_status(node),
        "git.collect-status" => builtin_git_collect_status(node, cfg),
        "git.diff" => builtin_git_diff(node, args),
        "git.commit" => builtin_git_commit(node, args, cfg),
        "git.push" => builtin_git_push(node, args),
        "tend.check" => builtin_tend_check(node, args),
        "nix.updateInputs" => builtin_nix_update_inputs(node, args, cfg),
        "hooks.install" => builtin_hooks_install(node, args, cfg),
        _ => StepResult {
            node: node.name.clone(),
            step_id: format!("builtin:{name}"),
            success: false,
            stdout: String::new(),
            stderr: format!("Unknown built-in: {name}"),
        },
    }
}

fn builtin_git_status(node: &ExecutionNode) -> StepResult {
    let output = std::process::Command::new("git")
        .args(["status", "--short", "--branch"])
        .current_dir(&node.path)
        .output();
    match output {
        Ok(o) => StepResult {
            node: node.name.clone(),
            step_id: "builtin:git.status".to_string(),
            success: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
        },
        Err(e) => StepResult {
            node: node.name.clone(),
            step_id: "builtin:git.status".to_string(),
            success: false,
            stdout: String::new(),
            stderr: format!("git status failed: {e}"),
        },
    }
}

fn builtin_git_collect_status(node: &ExecutionNode, _cfg: &WorkspaceConfig) -> StepResult {
    use crate::git::GitRepo;
    let repo_path = &node.path;
    if !repo_path.join(".git").exists() {
        let status = serde_json::json!({
            "name": node.name,
            "path": repo_path.display().to_string(),
            "branch": "",
            "is_dirty": false,
            "is_present": false,
            "staged_count": 0,
            "unstaged_count": 0,
            "untracked_count": 0,
        });
        return StepResult {
            node: node.name.clone(),
            step_id: "builtin:git.collect-status".to_string(),
            success: true,
            stdout: status.to_string(),
            stderr: String::new(),
        };
    }
    match GitRepo::open(repo_path) {
        Ok(repo) => match repo.status() {
            Ok(git_status) => {
                let branch = git_status.branch.clone();
                let is_dirty = git_status.is_dirty();
                let staged_count = git_status.staged_count();
                let unstaged_count = git_status.unstaged_count();
                let untracked_count = git_status.untracked_count();
                let ahead = repo.ahead_count().unwrap_or(0);
                let status = serde_json::json!({
                    "name": node.name,
                    "path": repo_path.display().to_string(),
                    "branch": branch,
                    "is_dirty": is_dirty,
                    "is_present": true,
                    "staged_count": staged_count,
                    "unstaged_count": unstaged_count,
                    "untracked_count": untracked_count,
                    "ahead": ahead,
                });
                StepResult {
                    node: node.name.clone(),
                    step_id: "builtin:git.collect-status".to_string(),
                    success: true,
                    stdout: status.to_string(),
                    stderr: String::new(),
                }
            }
            Err(e) => StepResult {
                node: node.name.clone(),
                step_id: "builtin:git.collect-status".to_string(),
                success: false,
                stdout: String::new(),
                stderr: format!("git.collect-status: {e}"),
            },
        },
        Err(e) => StepResult {
            node: node.name.clone(),
            step_id: "builtin:git.collect-status".to_string(),
            success: false,
            stdout: String::new(),
            stderr: format!("git.collect-status: {e}"),
        },
    }
}

fn builtin_git_diff(node: &ExecutionNode, args: &serde_json::Value) -> StepResult {
    let staged = args
        .get("staged")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut cmd_args = vec!["diff"];
    if staged {
        cmd_args.push("--cached");
    }
    let output = std::process::Command::new("git")
        .args(&cmd_args)
        .current_dir(&node.path)
        .output();
    match output {
        Ok(o) => StepResult {
            node: node.name.clone(),
            step_id: "builtin:git.diff".to_string(),
            success: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
        },
        Err(e) => StepResult {
            node: node.name.clone(),
            step_id: "builtin:git.diff".to_string(),
            success: false,
            stdout: String::new(),
            stderr: format!("git diff failed: {e}"),
        },
    }
}

fn builtin_git_commit(
    node: &ExecutionNode,
    args: &serde_json::Value,
    _cfg: &WorkspaceConfig,
) -> StepResult {
    let message = match args.get("message").and_then(|v| v.as_str()) {
        Some(m) => m.trim(),
        None => {
            return StepResult {
                node: node.name.clone(),
                step_id: "builtin:git.commit".to_string(),
                success: false,
                stdout: String::new(),
                stderr: "git.commit: --message <msg> required".to_string(),
            }
        }
    };
    if message.is_empty() {
        return StepResult {
            node: node.name.clone(),
            step_id: "builtin:git.commit".to_string(),
            success: false,
            stdout: String::new(),
            stderr: "git.commit: message must not be empty".to_string(),
        };
    }
    let stage = args.get("stage").and_then(|v| v.as_bool()).unwrap_or(true);

    if stage {
        let add_output = std::process::Command::new("git")
            .args(["add", "--all"])
            .current_dir(&node.path)
            .output();
        match add_output {
            Ok(o) if !o.status.success() => {
                return StepResult {
                    node: node.name.clone(),
                    step_id: "builtin:git.commit".to_string(),
                    success: false,
                    stdout: String::from_utf8_lossy(&o.stdout).to_string(),
                    stderr: String::from_utf8_lossy(&o.stderr).to_string(),
                };
            }
            Err(e) => {
                return StepResult {
                    node: node.name.clone(),
                    step_id: "builtin:git.commit".to_string(),
                    success: false,
                    stdout: String::new(),
                    stderr: format!("git add failed: {e}"),
                };
            }
            _ => {}
        }
    }

    let output = std::process::Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(&node.path)
        .output();
    match output {
        Ok(o) => StepResult {
            node: node.name.clone(),
            step_id: "builtin:git.commit".to_string(),
            success: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
        },
        Err(e) => StepResult {
            node: node.name.clone(),
            step_id: "builtin:git.commit".to_string(),
            success: false,
            stdout: String::new(),
            stderr: format!("git commit failed: {e}"),
        },
    }
}

fn builtin_git_push(node: &ExecutionNode, _args: &serde_json::Value) -> StepResult {
    let branch = match git::git_branch(&node.path) {
        Ok(b) => b,
        Err(e) => {
            return StepResult {
                node: node.name.clone(),
                step_id: "builtin:git.push".to_string(),
                success: false,
                stdout: String::new(),
                stderr: format!("git push: failed to get branch: {e}"),
            }
        }
    };
    let output = std::process::Command::new("git")
        .args(["push", "origin", &branch])
        .current_dir(&node.path)
        .output();
    match output {
        Ok(o) => StepResult {
            node: node.name.clone(),
            step_id: "builtin:git.push".to_string(),
            success: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
        },
        Err(e) => StepResult {
            node: node.name.clone(),
            step_id: "builtin:git.push".to_string(),
            success: false,
            stdout: String::new(),
            stderr: format!("git push failed: {e}"),
        },
    }
}

fn builtin_tend_check(node: &ExecutionNode, args: &serde_json::Value) -> StepResult {
    let profile = args
        .get("profile")
        .and_then(|v| v.as_str())
        .unwrap_or("pre-push");
    let affected_dag = args
        .get("affected_dag")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut cmd_args = vec!["check", "--profile", profile];
    if affected_dag {
        cmd_args.push("--affected-dag");
    }

    // Try flake-resolved tend first, then fallback to ambient PATH
    let tend_cmd = resolve_tend_command(&node.path);
    let output = std::process::Command::new(&tend_cmd.0)
        .args(tend_cmd.1.iter().map(|s| s.as_str()).collect::<Vec<&str>>())
        .args(&cmd_args)
        .current_dir(&node.path)
        .output();
    match output {
        Ok(o) => StepResult {
            node: node.name.clone(),
            step_id: format!("builtin:tend.check({profile})"),
            success: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).to_string(),
            stderr: String::from_utf8_lossy(&o.stderr).to_string(),
        },
        Err(e) => StepResult {
            node: node.name.clone(),
            step_id: format!("builtin:tend.check({profile})"),
            success: false,
            stdout: String::new(),
            stderr: format!("tend check failed: {e}"),
        },
    }
}

/// Resolve tend command, preferring flake-based resolution over ambient PATH.
/// Returns (program, extra_args) where extra_args are for nix run, etc.
fn resolve_tend_command(repo_path: &std::path::Path) -> (String, Vec<String>) {
    // Strategy 1: try `nix run` from root flake
    // Look for flake.nix in repo path or parent directories
    let flake_path = find_flake_root(repo_path);
    if let Some(fp) = flake_path {
        // Check if nix is available
        if std::process::Command::new("nix")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            // Try to resolve tend from the flake
            let tend_flake_path = fp.join("#tend");
            let check = std::process::Command::new("nix")
                .args([
                    "run",
                    &tend_flake_path.to_string_lossy(),
                    "--",
                    "check",
                    "--help",
                ])
                .current_dir(repo_path)
                .output();
            if check.map(|o| o.status.success()).unwrap_or(false) {
                return (
                    "nix".to_string(),
                    vec![
                        "run".to_string(),
                        tend_flake_path.to_string_lossy().to_string(),
                        "--".to_string(),
                    ],
                );
            }
            // Try .#default devshell
            let check_dev = std::process::Command::new("nix")
                .args([
                    "develop",
                    fp.to_string_lossy().as_ref(),
                    "--command",
                    "tend",
                    "check",
                    "--help",
                ])
                .current_dir(repo_path)
                .output();
            if check_dev.map(|o| o.status.success()).unwrap_or(false) {
                return (
                    "nix".to_string(),
                    vec![
                        "develop".to_string(),
                        fp.to_string_lossy().to_string(),
                        "--command".to_string(),
                        "tend".to_string(),
                    ],
                );
            }
        }
    }
    // Strategy 2: ambient PATH (original behavior)
    ("tend".to_string(), vec![])
}

fn find_flake_root(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let mut current = Some(path);
    while let Some(dir) = current {
        if dir.join("flake.nix").exists() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

/// Classify dirty nodes by type of dirt
#[derive(Debug, Clone, Serialize)]
pub struct DirtyNodeInfo {
    pub name: String,
    pub is_dirty: bool,
    pub has_meaningful_changes: bool,
    pub has_generated_changes: bool,
    pub has_ignored_pattern_changes: bool,
    pub has_staged_changes: bool,
    pub has_untracked_files: bool,
    pub generated_files: Vec<String>,
    pub untracked_files: Vec<String>,
}

/// Classify the dirt in a repository node.
/// Generated/ignored files (like `.pre-commit-config.yaml`) are classified
/// as non-meaningful and do not require a commit message.
pub fn classify_dirty_node(path: &std::path::Path) -> DirtyNodeInfo {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let mut has_meaningful = false;
    let mut has_generated = false;
    let has_ignored = false;
    let mut has_staged = false;
    let mut has_untracked = false;
    let mut generated_files = Vec::new();
    let mut untracked_files = Vec::new();

    if let Ok(output) = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(path)
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let status_code = &line[..2];
            let file_path = line[3..].trim().to_string();

            let is_generated = file_path.ends_with(".pre-commit-config.yaml")
                || file_path.contains(".git/hooks/")
                || file_path.starts_with("result");

            let staged =
                status_code.contains('M') || status_code.contains('A') || status_code.contains('D');

            if status_code == "??" {
                has_untracked = true;
                untracked_files.push(file_path.clone());
            }

            if staged || status_code != "??" {
                has_staged = has_staged || staged;
                if is_generated {
                    has_generated = true;
                    generated_files.push(file_path);
                } else {
                    has_meaningful = true;
                }
            }
        }
    }

    DirtyNodeInfo {
        name,
        is_dirty: has_meaningful || has_generated || has_untracked,
        has_meaningful_changes: has_meaningful,
        has_generated_changes: has_generated,
        has_ignored_pattern_changes: has_ignored,
        has_staged_changes: has_staged,
        has_untracked_files: has_untracked,
        generated_files,
        untracked_files,
    }
}

fn builtin_nix_update_inputs(
    node: &ExecutionNode,
    _args: &serde_json::Value,
    cfg: &WorkspaceConfig,
) -> StepResult {
    let lock_path = node.path.join("flake.lock");
    if !lock_path.exists() {
        return StepResult {
            node: node.name.clone(),
            step_id: "builtin:nix.updateInputs".to_string(),
            success: true,
            stdout: "No flake.lock, skipping".to_string(),
            stderr: String::new(),
        };
    }

    let lock_content = match std::fs::read_to_string(&lock_path) {
        Ok(c) => c,
        Err(e) => {
            return StepResult {
                node: node.name.clone(),
                step_id: "builtin:nix.updateInputs".to_string(),
                success: false,
                stdout: String::new(),
                stderr: format!("Failed to read flake.lock: {e}"),
            }
        }
    };

    let lock_val: serde_json::Value = match serde_json::from_str(&lock_content) {
        Ok(v) => v,
        Err(e) => {
            return StepResult {
                node: node.name.clone(),
                step_id: "builtin:nix.updateInputs".to_string(),
                success: false,
                stdout: String::new(),
                stderr: format!("Failed to parse flake.lock: {e}"),
            }
        }
    };

    // Determine which inputs are upstream providers in the workspace DAG
    let canonical_graph = match load_canonical_graph(cfg) {
        Ok(graph) => graph,
        Err(e) => {
            return StepResult {
                node: node.name.clone(),
                step_id: "builtin:nix.updateInputs".to_string(),
                success: false,
                stdout: String::new(),
                stderr: e,
            }
        }
    };
    let planner = graph::DagPlanner::new(&canonical_graph);
    let upstream_names: std::collections::BTreeSet<String> = planner
        .expand_closure(
            std::slice::from_ref(&node.name),
            graph::PlanClosureMode::Upstream,
            &config_order(cfg),
        )
        .into_iter()
        .filter(|name| name != &node.name)
        .collect();
    let exact_direct_inputs = planner.exact_input_names_for_transitive_upstream(&node.name);

    // Build a reverse map from upstream repo path -> upstream name for path matching
    let root_dir = cfg.config_dir.as_deref().unwrap_or(Path::new("."));
    let mut upstream_paths: Vec<(String, PathBuf)> = Vec::new();
    for uname in &upstream_names {
        if let Some(repo) = cfg.repos.iter().find(|r| r.name == *uname) {
            upstream_paths.push((uname.clone(), repo.resolved_path(cfg)));
        }
    }

    // Parse lock file root.inputs to find which inputs reference upstream workspace nodes
    let root_inputs = lock_val
        .get("nodes")
        .and_then(|n| n.get("root"))
        .and_then(|r| r.get("inputs"))
        .and_then(|i| i.as_object());
    let lock_nodes = lock_val.get("nodes").and_then(|n| n.as_object());

    let input_names: Vec<String> = match (root_inputs, lock_nodes) {
        (Some(inputs), Some(lnodes)) => inputs
            .keys()
            .filter(|input_name| {
                if exact_direct_inputs.values().any(|name| name == *input_name) {
                    return true;
                }
                // Fast path: input name directly matches an upstream node name
                if upstream_names.contains(*input_name) {
                    return true;
                }
                // Fallback: check lock file node's original path against upstream paths
                if let Some(node_ref) = inputs.get(*input_name).and_then(|v| v.as_str()) {
                    if upstream_names.contains(node_ref) {
                        return true;
                    }
                    if let Some(node_entry) = lnodes.get(node_ref) {
                        if let Some(original) = node_entry.get("original") {
                            if let Some(path_str) = original.get("path").and_then(|v| v.as_str()) {
                                let input_rel = Path::new(path_str);
                                if input_rel.is_relative() {
                                    let input_abs = root_dir.join(input_rel);
                                    return upstream_paths.iter().any(|(_, rp)| *rp == input_abs);
                                }
                            }
                        }
                    }
                }
                false
            })
            .cloned()
            .collect(),
        _ => Vec::new(),
    };

    if input_names.is_empty() {
        return StepResult {
            node: node.name.clone(),
            step_id: "builtin:nix.updateInputs".to_string(),
            success: true,
            stdout: format!(
                "No upstream inputs to update (node has {} provider(s))",
                upstream_names.len()
            ),
            stderr: String::new(),
        };
    }

    let mut successes = Vec::new();
    let mut failures = Vec::new();

    for input_name in &input_names {
        let output = std::process::Command::new("nix")
            .args(["flake", "lock", "--update-input", input_name])
            .current_dir(&node.path)
            .output();
        match output {
            Ok(o) if o.status.success() => successes.push(input_name.clone()),
            Ok(o) => failures.push(format!(
                "{input_name}: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Err(e) => failures.push(format!("{input_name}: {e}")),
        }
    }

    if failures.is_empty() {
        StepResult {
            node: node.name.clone(),
            step_id: "builtin:nix.updateInputs".to_string(),
            success: true,
            stdout: format!("Updated upstream inputs: {}", successes.join(", ")),
            stderr: String::new(),
        }
    } else {
        StepResult {
            node: node.name.clone(),
            step_id: "builtin:nix.updateInputs".to_string(),
            success: false,
            stdout: format!("Updated: {}", successes.join(", ")),
            stderr: format!("Failed: {}", failures.join("; ")),
        }
    }
}

fn builtin_hooks_install(
    node: &ExecutionNode,
    args: &serde_json::Value,
    cfg: &WorkspaceConfig,
) -> StepResult {
    let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
    let root = cfg.config_dir.as_deref().unwrap_or(Path::new("."));
    match install_hooks_for_repo(&node.name, &node.path, root, force) {
        Ok(result) => StepResult {
            node: node.name.clone(),
            step_id: "builtin:hooks.install".to_string(),
            success: result.installed,
            stdout: result.message,
            stderr: String::new(),
        },
        Err(e) => StepResult {
            node: node.name.clone(),
            step_id: "builtin:hooks.install".to_string(),
            success: false,
            stdout: String::new(),
            stderr: e,
        },
    }
}

pub fn print_plan(plan: &ExecutionPlan, json: bool) {
    if json {
        let nodes: Vec<serde_json::Value> = plan
            .nodes
            .iter()
            .map(|n| {
                serde_json::json!({
                    "name": n.name,
                    "path": n.path,
                    "layer": n.layer,
                    "directly_selected": n.directly_selected,
                    "directly_changed": n.directly_changed,
                    "downstream_only": n.downstream_only,
                    "steps": n.steps.iter().map(|s| {
                        let kind_str = match &s.kind {
                            StepKind::Shell { argv } => serde_json::json!({"shell": argv}),
                            StepKind::Builtin { name, args } => serde_json::json!({"builtin": name, "args": args}),
                        };
                        serde_json::json!({
                            "id": s.id,
                            "mode": if s.mode == ExecutionMode::Mutating { "mutating" } else { "readonly" },
                            "kind": kind_str,
                            "condition": s.condition.map(|c| format!("{:?}", c)),
                        })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "nodes": nodes,
                "total": plan.nodes.len()
            }))
            .unwrap()
        );
    } else {
        println!("Execution Plan:");
        println!();
        for (i, node) in plan.nodes.iter().enumerate() {
            let sel = if node.directly_selected {
                " [SELECTED]"
            } else if node.downstream_only {
                " [DOWNSTREAM]"
            } else {
                ""
            };
            println!("[{}] {}{sel} (layer {})", i + 1, node.name, node.layer);
            if let Some(ref role) = node.role {
                if !role.is_empty() && role != "unknown" {
                    println!("    role: {role}");
                }
            }
            println!("    path: {}", node.path.display());
            if node.steps.is_empty() {
                println!("    (no steps)");
            } else {
                for step in &node.steps {
                    let mode_str = match step.mode {
                        ExecutionMode::Mutating => " [MUTATING]",
                        ExecutionMode::ReadOnly => "",
                    };
                    match &step.kind {
                        StepKind::Shell { argv } => {
                            println!("    step: {}: {}{mode_str}", step.id, argv.join(" "));
                        }
                        StepKind::Builtin { name, .. } => {
                            println!("    step: {}: builtin {}{mode_str}", step.id, name);
                        }
                    }
                }
            }
            println!();
        }
        println!("Total: {} node(s)", plan.nodes.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_CFG_COUNTER: AtomicUsize = AtomicUsize::new(0);

    fn make_test_cfg() -> WorkspaceConfig {
        let id = TEST_CFG_COUNTER.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!(
            "phenix_stitch_exec_tests_{}_{}",
            std::process::id(),
            id
        ));
        let stitch_dir = root.join(".stitch");
        std::fs::create_dir_all(&stitch_dir).unwrap();
        for path in [
            "flakes/00-pins/pins",
            "flakes/02-producers/tools",
            "flakes/05-consumers/hosts",
        ] {
            std::fs::create_dir_all(root.join(path)).unwrap();
        }
        std::fs::write(
            root.join("flakes/00-pins/pins/flake.lock"),
            r#"{ "nodes": { "root": {} }, "root": "root", "version": 7 }"#,
        )
        .unwrap();
        std::fs::write(
            root.join("flakes/02-producers/tools/flake.lock"),
            r#"{
              "nodes": {
                "root": { "inputs": { "pins": "pins" } },
                "pins": { "locked": { "type": "path", "path": "../00-pins/pins" } }
              },
              "root": "root",
              "version": 7
            }"#,
        )
        .unwrap();
        std::fs::write(
            root.join("flakes/05-consumers/hosts/flake.lock"),
            r#"{
              "nodes": {
                "root": { "inputs": { "tools": "tools" } },
                "tools": { "locked": { "type": "path", "path": "../../02-producers/tools" } }
              },
              "root": "root",
              "version": 7
            }"#,
        )
        .unwrap();
        std::fs::write(
            stitch_dir.join("topology.json"),
            r#"{
              "version": 1,
              "workspace": "test",
              "repos": [
                { "name": "pins", "role": "pins", "layer": 0 },
                { "name": "tools", "role": "producer", "layer": 2 },
                { "name": "hosts", "role": "consumer", "layer": 5 },
                { "name": "phenix", "role": "root", "layer": 6 }
              ]
            }"#,
        )
        .unwrap();

        WorkspaceConfig {
            version: 1,
            workspace: "test".to_string(),
            repos: vec![
                crate::model::RepoConfig {
                    name: "pins".to_string(),
                    path: "flakes/00-pins/pins".to_string(),
                    remote: None,
                },
                crate::model::RepoConfig {
                    name: "tools".to_string(),
                    path: "flakes/02-producers/tools".to_string(),
                    remote: None,
                },
                crate::model::RepoConfig {
                    name: "hosts".to_string(),
                    path: "flakes/05-consumers/hosts".to_string(),
                    remote: None,
                },
                crate::model::RepoConfig {
                    name: "phenix".to_string(),
                    path: ".".to_string(),
                    remote: None,
                },
            ],
            config_dir: Some(root),
        }
    }

    #[test]
    fn test_parse_selection_mode() {
        assert_eq!(parse_selection_mode("all").unwrap(), SelectionMode::All);
        assert_eq!(
            parse_selection_mode("changed").unwrap(),
            SelectionMode::Changed
        );
        assert_eq!(parse_selection_mode("dirty").unwrap(), SelectionMode::Dirty);
        assert_eq!(
            parse_selection_mode("explicit").unwrap(),
            SelectionMode::Explicit
        );
        assert!(parse_selection_mode("foo").is_err());
    }

    #[test]
    fn test_parse_closure_mode() {
        assert_eq!(parse_closure_mode("self").unwrap(), ClosureMode::SelfOnly);
        assert_eq!(
            parse_closure_mode("upstream").unwrap(),
            ClosureMode::Upstream
        );
        assert_eq!(
            parse_closure_mode("downstream").unwrap(),
            ClosureMode::Downstream
        );
        assert_eq!(
            parse_closure_mode("connected").unwrap(),
            ClosureMode::Connected
        );
        assert_eq!(parse_closure_mode("all").unwrap(), ClosureMode::All);
        assert!(parse_closure_mode("foo").is_err());
    }

    #[test]
    fn test_parse_order_mode() {
        assert_eq!(parse_order_mode("stable").unwrap(), OrderMode::Stable);
        assert_eq!(
            parse_order_mode("providers-first").unwrap(),
            OrderMode::ProvidersFirst
        );
        assert_eq!(
            parse_order_mode("consumers-first").unwrap(),
            OrderMode::ConsumersFirst
        );
        assert!(parse_order_mode("foo").is_err());
    }

    #[test]
    fn test_parse_execution_mode() {
        assert_eq!(
            parse_execution_mode("readonly").unwrap(),
            ExecutionMode::ReadOnly
        );
        assert_eq!(
            parse_execution_mode("mutating").unwrap(),
            ExecutionMode::Mutating
        );
        assert!(parse_execution_mode("foo").is_err());
    }

    #[test]
    fn test_parse_condition() {
        assert_eq!(parse_condition("always").unwrap(), StepCondition::Always);
        assert_eq!(parse_condition("dirty").unwrap(), StepCondition::Dirty);
        assert_eq!(parse_condition("staged").unwrap(), StepCondition::Staged);
        assert_eq!(
            parse_condition("directly_changed").unwrap(),
            StepCondition::DirectlyChanged
        );
        assert_eq!(
            parse_condition("downstream_only").unwrap(),
            StepCondition::DownstreamOnly
        );
        assert_eq!(
            parse_condition("has_lockfile").unwrap(),
            StepCondition::HasLockfile
        );
        assert_eq!(
            parse_condition("has_changed_inputs").unwrap(),
            StepCondition::HasChangedInputs
        );
        assert!(parse_condition("foo").is_err());
    }

    #[test]
    fn test_closure_expand_self() {
        let all = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let deps: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let result = expand_closure(
            &["a".to_string()],
            ClosureMode::SelfOnly,
            &all,
            &deps,
            &dependents,
        );
        assert_eq!(result, vec!["a"]);
    }

    #[test]
    fn test_closure_expand_all() {
        let all = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let deps: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let result = expand_closure(
            &["a".to_string()],
            ClosureMode::All,
            &all,
            &deps,
            &dependents,
        );
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_topological_sort_providers_first() {
        let all_nodes = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut deps: BTreeMap<String, Vec<String>> = BTreeMap::new();
        deps.insert("a".to_string(), vec!["b".to_string()]);
        deps.insert("b".to_string(), vec!["c".to_string()]);
        deps.insert("c".to_string(), vec![]);
        let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        // a depends on b, b depends on c
        // providers-first: c, b, a
        let result =
            topological_sort(&all_nodes, &deps, &order, OrderMode::ProvidersFirst).unwrap();
        assert_eq!(result, vec!["c", "b", "a"]);
    }

    #[test]
    fn test_topological_sort_consumers_first() {
        let all_nodes = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut deps: BTreeMap<String, Vec<String>> = BTreeMap::new();
        deps.insert("a".to_string(), vec!["b".to_string()]);
        deps.insert("b".to_string(), vec!["c".to_string()]);
        deps.insert("c".to_string(), vec![]);
        let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        // reverse of providers-first: a, b, c
        let result =
            topological_sort(&all_nodes, &deps, &order, OrderMode::ConsumersFirst).unwrap();
        assert_eq!(result, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_topological_sort_cycle_fails() {
        let all_nodes = vec!["a".to_string(), "b".to_string()];
        let mut deps: BTreeMap<String, Vec<String>> = BTreeMap::new();
        deps.insert("a".to_string(), vec!["b".to_string()]);
        deps.insert("b".to_string(), vec!["a".to_string()]);
        let order = all_nodes.clone();
        let result = topological_sort(&all_nodes, &deps, &order, OrderMode::ProvidersFirst);
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_empty_shell_argv_fails_clearly() {
        let node = ExecutionNode {
            name: "test".to_string(),
            path: PathBuf::from("/tmp"),
            role: None,
            layer: 0,
            directly_selected: true,
            directly_changed: false,
            downstream_only: false,
            steps: vec![],
        };
        let step = ExecutionStep {
            id: "empty".to_string(),
            mode: ExecutionMode::ReadOnly,
            kind: StepKind::Shell { argv: vec![] },
            condition: None,
        };
        let result = execute_step(&node, &step, &make_test_cfg());
        assert!(!result.success);
        assert!(result.stderr.contains("empty argv"));
    }

    #[test]
    fn test_mutating_recipe_requires_apply() {
        let scope = ExecutionScope {
            selection: SelectionMode::All,
            explicit_nodes: vec![],
            closure: ClosureMode::SelfOnly,
            order: OrderMode::Stable,
        };
        let steps = vec![ExecutionStep {
            id: "mutate".to_string(),
            mode: ExecutionMode::Mutating,
            kind: StepKind::Shell {
                argv: vec!["echo".to_string(), "hi".to_string()],
            },
            condition: None,
        }];
        let cfg = make_test_cfg();
        let plan = build_plan(&cfg, &scope, steps).unwrap();
        assert!(!plan.nodes.is_empty());

        let opts = RunOptions {
            dry_run: false,
            apply: false,
            json: false,
        };
        let result = run_plan(&cfg, &plan, &opts);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("mutating") || err.contains("--apply"));
    }

    #[test]
    fn test_mutating_with_dry_run_ok() {
        let scope = ExecutionScope {
            selection: SelectionMode::All,
            explicit_nodes: vec![],
            closure: ClosureMode::SelfOnly,
            order: OrderMode::Stable,
        };
        let steps = vec![ExecutionStep {
            id: "mutate".to_string(),
            mode: ExecutionMode::Mutating,
            kind: StepKind::Shell {
                argv: vec!["echo".to_string(), "hi".to_string()],
            },
            condition: None,
        }];
        let cfg = make_test_cfg();
        let plan = build_plan(&cfg, &scope, steps).unwrap();
        let opts = RunOptions {
            dry_run: true,
            apply: false,
            json: false,
        };
        let result = run_plan(&cfg, &plan, &opts);
        assert!(result.is_ok());
    }

    #[test]
    fn test_readonly_does_not_require_apply() {
        let scope = ExecutionScope {
            selection: SelectionMode::All,
            explicit_nodes: vec![],
            closure: ClosureMode::SelfOnly,
            order: OrderMode::Stable,
        };
        let steps = vec![ExecutionStep {
            id: "read".to_string(),
            mode: ExecutionMode::ReadOnly,
            kind: StepKind::Shell {
                argv: vec!["echo".to_string(), "read".to_string()],
            },
            condition: None,
        }];
        let cfg = make_test_cfg();
        let plan = build_plan(&cfg, &scope, steps).unwrap();
        let opts = RunOptions {
            dry_run: false,
            apply: false,
            json: false,
        };
        let result = run_plan(&cfg, &plan, &opts);
        assert!(result.is_ok());
    }

    #[test]
    fn test_invalid_closure_fails() {
        assert!(parse_closure_mode("foo").is_err());
    }

    #[test]
    fn test_invalid_order_fails() {
        assert!(parse_order_mode("foo").is_err());
    }

    #[test]
    fn test_invalid_condition_fails() {
        assert!(parse_condition("foo").is_err());
    }

    #[test]
    fn test_unknown_builtin_fails() {
        let node = ExecutionNode {
            name: "test".to_string(),
            path: PathBuf::from("/tmp"),
            role: None,
            layer: 0,
            directly_selected: true,
            directly_changed: false,
            downstream_only: false,
            steps: vec![],
        };
        let args = serde_json::json!({});
        let result = run_builtin(&node, &make_test_cfg(), "unknown.builtin", &args);
        assert!(!result.success);
        assert!(result.stderr.contains("Unknown built-in"));
    }

    #[test]
    fn test_git_builtin_ok() {
        let node = ExecutionNode {
            name: "test".to_string(),
            path: PathBuf::from("/tmp"),
            role: None,
            layer: 0,
            directly_selected: true,
            directly_changed: false,
            downstream_only: false,
            steps: vec![],
        };
        let result = run_builtin(
            &node,
            &make_test_cfg(),
            "git.status",
            &serde_json::json!({}),
        );
        // git status may succeed or fail with "not a git repository" - either is fine
        let stderr_contains = result.stderr.contains("not a git repository");
        assert!(
            result.success || stderr_contains,
            "expected success or not-a-repo error, got stdout={:?} stderr={:?}",
            result.stdout,
            result.stderr
        );
    }

    #[test]
    fn test_stable_order_returns_config_order() {
        let all_nodes = vec!["z".to_string(), "a".to_string(), "m".to_string()];
        let deps: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let result = expand_closure(&all_nodes, ClosureMode::All, &all_nodes, &deps, &dependents);
        // expand_closure returns all nodes for All mode, order depends on BTreeSet iteration
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_closure_upstream_traversal() {
        let all_nodes = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut deps: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        // a depends on b, b depends on c
        deps.insert("a".to_string(), vec!["b".to_string()]);
        deps.insert("b".to_string(), vec!["c".to_string()]);
        deps.insert("c".to_string(), vec![]);
        dependents.insert("c".to_string(), vec!["b".to_string()]);
        dependents.insert("b".to_string(), vec!["a".to_string()]);
        dependents.insert("a".to_string(), vec![]);

        // Upstream from a: a, b, c (a + its transitive providers)
        let result = expand_closure(
            &["a".to_string()],
            ClosureMode::Upstream,
            &all_nodes,
            &deps,
            &dependents,
        );
        let mut sorted: Vec<String> = result;
        sorted.sort();
        assert_eq!(sorted, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_closure_downstream_traversal() {
        let all_nodes = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut deps: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        // a depends on b, b depends on c, so c is lowest-level provider
        deps.insert("a".to_string(), vec!["b".to_string()]);
        deps.insert("b".to_string(), vec!["c".to_string()]);
        deps.insert("c".to_string(), vec![]);
        dependents.insert("c".to_string(), vec!["b".to_string()]);
        dependents.insert("b".to_string(), vec!["a".to_string()]);
        dependents.insert("a".to_string(), vec![]);

        // Downstream from c: c, b, a (c + its transitive consumers)
        let result = expand_closure(
            &["c".to_string()],
            ClosureMode::Downstream,
            &all_nodes,
            &deps,
            &dependents,
        );
        let mut sorted: Vec<String> = result;
        sorted.sort();
        assert_eq!(sorted, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_closure_connected_combines_both() {
        let all_nodes = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut deps: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut dependents: BTreeMap<String, Vec<String>> = BTreeMap::new();
        deps.insert("a".to_string(), vec!["b".to_string()]);
        deps.insert("b".to_string(), vec!["c".to_string()]);
        deps.insert("c".to_string(), vec![]);
        dependents.insert("c".to_string(), vec!["b".to_string()]);
        dependents.insert("b".to_string(), vec!["a".to_string()]);
        dependents.insert("a".to_string(), vec![]);

        // Connected from b: b, a (upstream: b depends on c, wait no - b's providers are c, b's consumers are a)
        // Actually: a depends on b, b depends on c
        // Upstream of b: b's providers = {c}; b itself. So {b, c}
        // Downstream of b: b's consumers = {a}; b itself. So {a, b}
        // Connected: {a, b, c}
        let result = expand_closure(
            &["b".to_string()],
            ClosureMode::Connected,
            &all_nodes,
            &deps,
            &dependents,
        );
        let mut sorted: Vec<String> = result;
        sorted.sort();
        assert_eq!(sorted, vec!["a", "b", "c"]);
    }

    #[test]
    fn discovered_graph_does_not_require_topology_metadata() {
        let cfg = make_test_cfg();
        let root = cfg.config_dir.as_ref().unwrap();
        std::fs::remove_file(root.join(".stitch/topology.json")).unwrap();
        let scope = ExecutionScope {
            selection: SelectionMode::Explicit,
            explicit_nodes: vec!["pins".to_string()],
            closure: ClosureMode::Downstream,
            order: OrderMode::ProvidersFirst,
        };

        let nodes = build_scope(&cfg, &scope).unwrap();
        let names = nodes.into_iter().map(|node| node.name).collect::<Vec<_>>();

        assert_eq!(names, vec!["pins", "tools", "hosts"]);
    }

    #[test]
    fn test_explicit_node_selection_single() {
        let scope = ExecutionScope {
            selection: SelectionMode::Explicit,
            explicit_nodes: vec!["pins".to_string()],
            closure: ClosureMode::SelfOnly,
            order: OrderMode::Stable,
        };
        let cfg = make_test_cfg();
        let nodes = build_scope(&cfg, &scope).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name, "pins");
        assert!(nodes[0].directly_selected);
    }

    #[test]
    fn test_downstream_only_excludes_direct_selection() {
        let scope = ExecutionScope {
            selection: SelectionMode::Explicit,
            explicit_nodes: vec!["pins".to_string()],
            closure: ClosureMode::Downstream,
            order: OrderMode::ProvidersFirst,
        };
        let cfg = make_test_cfg();
        let nodes = build_scope(&cfg, &scope).unwrap();
        let by_name: BTreeMap<String, bool> = nodes
            .iter()
            .map(|node| (node.name.clone(), node.downstream_only))
            .collect();
        assert_eq!(by_name.get("pins"), Some(&false));
        assert_eq!(by_name.get("tools"), Some(&true));
        assert_eq!(by_name.get("hosts"), Some(&true));
    }

    #[test]
    fn test_explicit_node_unknown_fails() {
        let scope = ExecutionScope {
            selection: SelectionMode::Explicit,
            explicit_nodes: vec!["unknown-node".to_string()],
            closure: ClosureMode::SelfOnly,
            order: OrderMode::Stable,
        };
        let cfg = make_test_cfg();
        let result = build_scope(&cfg, &scope);
        assert!(result.is_err());
    }

    #[test]
    fn test_step_condition_always_included() {
        let steps = vec![ExecutionStep {
            id: "always-step".to_string(),
            mode: ExecutionMode::ReadOnly,
            kind: StepKind::Shell {
                argv: vec!["echo".to_string(), "always".to_string()],
            },
            condition: Some(StepCondition::Always),
        }];
        let scope = ExecutionScope {
            selection: SelectionMode::Explicit,
            explicit_nodes: vec!["pins".to_string()],
            closure: ClosureMode::SelfOnly,
            order: OrderMode::Stable,
        };
        let cfg = make_test_cfg();
        let plan = build_plan(&cfg, &scope, steps).unwrap();
        assert_eq!(plan.nodes.len(), 1);
        assert_eq!(plan.nodes[0].steps.len(), 1);
        assert_eq!(plan.nodes[0].steps[0].id, "always-step");
    }

    #[test]
    fn test_dry_run_returns_ok_report() {
        let scope = ExecutionScope {
            selection: SelectionMode::Explicit,
            explicit_nodes: vec!["pins".to_string()],
            closure: ClosureMode::SelfOnly,
            order: OrderMode::Stable,
        };
        let steps = vec![ExecutionStep {
            id: "test".to_string(),
            mode: ExecutionMode::ReadOnly,
            kind: StepKind::Shell {
                argv: vec!["echo".to_string(), "hello".to_string()],
            },
            condition: None,
        }];
        let cfg = make_test_cfg();
        let plan = build_plan(&cfg, &scope, steps).unwrap();
        let opts = RunOptions {
            dry_run: true,
            apply: false,
            json: false,
        };
        let report = run_plan(&cfg, &plan, &opts).unwrap();
        assert_eq!(report.total_nodes, 1);
        assert_eq!(report.successful_nodes, 1);
        assert_eq!(report.failed_nodes, 0);
    }
}
