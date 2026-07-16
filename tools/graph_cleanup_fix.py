#!/usr/bin/env python3
"""Finalize the one-way collapse to Stitch's current graph API."""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def rewrite(path: str, transform) -> None:
    target = ROOT / path
    source = target.read_text()
    updated = transform(source)
    if updated != source:
        target.write_text(updated)


def replace(path: str, old: str, new: str) -> None:
    rewrite(path, lambda source: source.replace(old, new))


(ROOT / "crates/stitch/src/lib.rs").write_text(
    """pub mod changeset;
pub mod config;
pub mod exec;
pub mod git;
pub mod graph;
pub mod model;
pub mod recipe;
pub mod status;
pub mod sync;
pub(crate) mod time;
pub mod validate;
pub mod workloop;
pub mod workspace;

pub use exec::{
    build_plan, build_scope, parse_closure_mode, parse_execution_mode, parse_order_mode,
    parse_selection_mode, run_plan, ClosureMode, ExecutionMode, ExecutionNode, ExecutionPlan,
    ExecutionScope, ExecutionStep, OrderMode, RunOptions, SelectionMode, StepCondition, StepKind,
    StepResult,
};
pub use graph::{
    derive_workspace_graph, derive_workspace_graph_from_config, discover_inventory,
    discover_inventory_from_config, parse_flake_lock, provider_before_consumer_order,
    validate_graph, CanonicalWorkspaceGraph, CanonicalizeError, DagGenerationStrategy, DagPlan,
    DagPlanRequest, DagPlanner, DiagnosticSeverity, EdgeKind, EdgeSpec, ExternalInput,
    GenerationContext, GraphDiagnostic, GraphValidationReport, NodeKind, NodeSpec,
    PlanClosureMode, PlanOrderMode, PlanSelectionMode, PlannedDagNode, RenderFormat, RepoRole,
    StrategyError, ValidateOptions, WorkspaceDiscovery, WorkspaceGraphDraft,
};
"""
)

replace(
    "crates/stitch/src/sync.rs",
    "crate::graph::discover_graph(cfg)?",
    "crate::graph::discover_sync_graph(cfg)?",
)


def repair_render(source: str) -> str:
    canonical_graph_overload = re.compile(
        r"\nfn render_graph_snapshot\(\n"
        r"    graph: &CanonicalWorkspaceGraph,\n"
        r"    format: RenderFormat,\n"
        r"\) -> Result<String, String> \{\n"
        r"    render_graph_snapshot\(&graph\.to_snapshot\(\), format\)\n"
        r"\}\n",
    )
    canonical_order_overload = re.compile(
        r"\nfn render_order_snapshot\(\n"
        r"    graph: &CanonicalWorkspaceGraph,\n"
        r"    order: &\[String\],\n"
        r"    format: RenderFormat,\n"
        r"\) -> Result<String, String> \{\n"
        r"    render_order_snapshot\(&graph\.to_snapshot\(\), order, format\)\n"
        r"\}\n",
    )
    source = canonical_graph_overload.sub("\n", source)
    source = canonical_order_overload.sub("\n", source)
    source = source.replace("edge.reason", "edge.kind")
    manual_arm = '                super::EdgeKind::Manual { .. } => "manual".to_string(),\n'
    submodule_arm = (
        '                super::EdgeKind::SubmoduleMembership { .. } => '
        '"submodule-membership".to_string(),\n'
    )
    if manual_arm in source and submodule_arm not in source:
        source = source.replace(manual_arm, manual_arm + submodule_arm)
    return source


rewrite("crates/stitch/src/graph/render.rs", repair_render)


def repair_validate(source: str) -> str:
    source = source.replace("reason: EdgeKind::", "kind: EdgeKind::")
    source = source.replace(
        "CanonicalWorkspaceGraph::from_snapshot(graph)",
        "CanonicalWorkspaceGraph::from_draft(graph)",
    )
    source = source.replace(
        "validate_canonical_graph(&canonical",
        "validate_graph(&canonical",
    )
    source = source.replace(
        "pub fn validate_snapshot(\n"
        "    graph: &crate::graph::CanonicalWorkspaceGraph,\n",
        "pub fn validate_graph(\n"
        "    graph: &crate::graph::CanonicalWorkspaceGraph,\n",
    )
    return source


rewrite("crates/stitch/src/graph/validate.rs", repair_validate)

# Current graph derivation returns the canonical graph directly. No retired
# names or adapter vocabulary may remain in production Rust code.
retired = re.compile(
    r"\bWorkspaceDag\b|\bWorkspaceNode\b|\bWorkspaceEdge\b|"
    r"\bFlakeNode\b|\bDependencyEdge\b|from_legacy|to_legacy"
)
for path in (ROOT / "crates").rglob("*.rs"):
    if retired.search(path.read_text()):
        raise SystemExit(f"retired graph API remains in {path.relative_to(ROOT)}")

render = (ROOT / "crates/stitch/src/graph/render.rs").read_text()
if render.count("fn render_graph_snapshot(") != 1:
    raise SystemExit("render_graph_snapshot must have exactly one draft implementation")
if render.count("fn render_order_snapshot(") != 1:
    raise SystemExit("render_order_snapshot must have exactly one draft implementation")

validate = (ROOT / "crates/stitch/src/graph/validate.rs").read_text()
if validate.count("fn validate_snapshot(") != 1:
    raise SystemExit("validate_snapshot must have exactly one private implementation")
if validate.count("pub fn validate_graph(") != 1:
    raise SystemExit("validate_graph must have exactly one canonical public entry point")
