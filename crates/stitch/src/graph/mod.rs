use serde::{Deserialize, Serialize};

pub mod canonical;
pub mod derive;
pub mod inventory;
pub mod lock;
pub mod planner;
pub mod render;
pub mod spec;
pub mod strategy;
pub mod topo;
pub mod validate;

pub use canonical::{CanonicalWorkspaceGraph, CanonicalizeError};
pub use derive::{derive_workspace_graph, derive_workspace_graph_from_config};
pub use inventory::{discover_inventory, discover_inventory_from_config, WorkspaceDiscovery};
pub use lock::parse_flake_lock;
pub use planner::{
    DagPlan, DagPlanRequest, DagPlanner, PlanClosureMode, PlanOrderMode, PlanSelectionMode,
    PlannedDagNode,
};
pub use render::RenderFormat;
pub use spec::{
    DagGenerationStrategy, EdgeKind, EdgeSpec, GenerationContext, NodeSpec, StrategyError,
    WorkspaceGraphDraft,
};
pub use strategy::{CompositeDagGenerationStrategy, FlakeLocksStrategy, GitSubmodulesStrategy};
pub use topo::provider_before_consumer_order;
pub use validate::{
    validate_graph, DiagnosticSeverity, GraphDiagnostic, GraphValidationReport, ValidateOptions,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Pins,
    PackageProvider,
    ToolProvider,
    ShellProvider,
    DesktopProvider,
    HostConsumer,
    WorkspaceRoot,
    External,
    Unknown,
}

impl NodeKind {
    pub fn is_provider(&self) -> bool {
        matches!(
            self,
            Self::Pins
                | Self::PackageProvider
                | Self::ToolProvider
                | Self::ShellProvider
                | Self::DesktopProvider
        )
    }

    pub fn is_consumer(&self) -> bool {
        matches!(self, Self::HostConsumer)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepoRole {
    Pins,
    Lib,
    PkgsBase,
    Protocols,
    Producer,
    Integration,
    PkgsAggregator,
    Consumer,
    Root,
    External,
    Unknown,
}

impl RepoRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pins => "pins",
            Self::Lib => "lib",
            Self::PkgsBase => "pkgs-base",
            Self::Protocols => "protocols",
            Self::Producer => "producer",
            Self::Integration => "integration",
            Self::PkgsAggregator => "pkgs-aggregator",
            Self::Consumer => "consumer",
            Self::Root => "root",
            Self::External => "external",
            Self::Unknown => "unknown",
        }
    }

    pub fn layer(self) -> Option<u32> {
        Some(match self {
            Self::Pins => 0,
            Self::Lib | Self::PkgsBase | Self::Protocols => 1,
            Self::Producer => 2,
            Self::Integration => 3,
            Self::PkgsAggregator => 4,
            Self::Consumer => 5,
            Self::Root => 6,
            Self::External => 255,
            Self::Unknown => return None,
        })
    }

    pub fn is_root(self) -> bool {
        matches!(self, Self::Root)
    }

    pub fn is_producer(self) -> bool {
        self == Self::Producer
    }

    pub fn is_consumer(self) -> bool {
        self == Self::Consumer
    }

    pub fn is_pkgs_aggregator(self) -> bool {
        self == Self::PkgsAggregator
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalInput {
    pub owner_node: String,
    pub input_name: String,
    pub locked_type: Option<String>,
    pub url_or_repo: Option<String>,
    pub rev: Option<String>,
}
