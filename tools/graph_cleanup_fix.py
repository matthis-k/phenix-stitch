#!/usr/bin/env python3
"""Repair consumers after collapsing Stitch's graph API generations."""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace(path: str, old: str, new: str) -> None:
    target = ROOT / path
    text = target.read_text()
    updated = text.replace(old, new)
    if updated != text:
        target.write_text(updated)


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
replace("crates/stitch/src/graph/render.rs", "edge.reason", "edge.kind")
replace("crates/stitch/src/graph/validate.rs", "reason: EdgeKind::", "kind: EdgeKind::")
replace(
    "crates/stitch/src/graph/validate.rs",
    "CanonicalWorkspaceGraph::from_snapshot(graph)",
    "CanonicalWorkspaceGraph::from_draft(graph)",
)
replace(
    "crates/stitch/src/graph/validate.rs",
    "validate_canonical_graph(&canonical",
    "validate_graph(&canonical",
)
