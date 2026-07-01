pub mod changeset;
pub mod config;
pub mod exec;
pub mod git;
pub mod graph;
pub mod model;
pub mod recipe;
pub mod status;
pub mod sync;
pub mod validate;

pub use exec::{
    build_plan, build_scope, parse_closure_mode, parse_execution_mode, parse_order_mode,
    parse_selection_mode, run_plan, ClosureMode, ExecutionMode, ExecutionNode, ExecutionPlan,
    ExecutionScope, ExecutionStep, OrderMode, RunOptions, SelectionMode, StepCondition, StepKind,
    StepResult,
};
pub use graph::{
    canonical::{CanonicalWorkspaceGraph, CanonicalizeError},
    derive::derive_graph_from_locks,
    inventory::{discover_inventory, InventoryOptions},
    lock::parse_flake_lock,
    planner::{
        DagPlan, DagPlanRequest, DagPlanner, PlanClosureMode, PlanOrderMode, PlanSelectionMode,
        PlannedDagNode,
    },
    render::RenderFormat,
    spec::{
        DagGenerationStrategy, EdgeKind, EdgeSpec, GenerationContext, NodeSpec, StrategyError,
        WorkspaceGraphDraft,
    },
    topo::provider_before_consumer_order,
    validate::{
        validate_canonical_graph, validate_graph, DiagnosticSeverity, GraphDiagnostic,
        GraphValidationReport, ValidateOptions,
    },
    EdgeReason, ExternalInput, GraphSource, NodeKind, RepoRole, WorkspaceDag, WorkspaceEdge,
    WorkspaceNode,
};
