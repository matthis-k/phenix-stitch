use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::graph::inventory::{
    discover_inventory, discover_inventory_from_config, WorkspaceDiscovery,
};
use crate::graph::lock::{
    build_workspace_aliases, external_from_lock, input_target_name, map_lock_target_to_workspace,
    parse_flake_lock,
};
use crate::graph::spec::{
    DagGenerationStrategy, EdgeKind, EdgeSpec, GenerationContext, NodeSpec, StrategyError,
    WorkspaceGraphDraft,
};
use crate::graph::validate::GraphDiagnostic;
use crate::graph::NodeKind;
use crate::model::WorkspaceConfig;

#[derive(Debug, Default)]
pub struct FlakeLocksStrategy;

impl FlakeLocksStrategy {
    pub fn generate_from_config(
        &self,
        config: &WorkspaceConfig,
        metadata: Option<&Path>,
    ) -> Result<WorkspaceGraphDraft, StrategyError> {
        let discovery = discover_inventory_from_config(config, metadata)
            .map_err(|error| StrategyError::new(self.name(), error))?;
        self.generate_from_discovery(discovery)
    }

    fn generate_from_discovery(
        &self,
        discovery: WorkspaceDiscovery,
    ) -> Result<WorkspaceGraphDraft, StrategyError> {
        let aliases = build_workspace_aliases(&discovery.nodes);
        let nodes: BTreeMap<String, NodeSpec> = discovery.nodes.into_iter().collect();
        let mut draft = WorkspaceGraphDraft::new(nodes);
        let node_ids: Vec<String> = draft.nodes.keys().cloned().collect();

        for node_id in &node_ids {
            let node = draft.nodes.get(node_id).expect("node id from keys");
            let lock_path = node.path.join("flake.lock");
            if !lock_path.exists() {
                draft.diagnostics.push(GraphDiagnostic::warn(
                    "missing_flake_lock",
                    format!(
                        "node '{node_id}' has no flake.lock at {}",
                        lock_path.display()
                    ),
                    vec![node_id.clone()],
                ));
                continue;
            }

            let lock = match parse_flake_lock(&lock_path) {
                Ok(lock) => lock,
                Err(error) => {
                    draft.diagnostics.push(GraphDiagnostic::error(
                        "parse_flake_lock_failed",
                        format!("node '{node_id}': {error}"),
                        vec![node_id.clone()],
                    ));
                    continue;
                }
            };
            let root_lock_node_name = lock.root.as_deref().unwrap_or("root");
            let Some(root_lock_node) = lock.nodes.get(root_lock_node_name) else {
                draft.diagnostics.push(GraphDiagnostic::error(
                    "lock_root_node_missing",
                    format!("node '{node_id}': lock root node '{root_lock_node_name}' not found"),
                    vec![node_id.clone()],
                ));
                continue;
            };
            let Some(inputs) = &root_lock_node.inputs else {
                continue;
            };

            for (input_name, input_value) in inputs {
                let Some(lock_target_name) = input_target_name(input_value) else {
                    continue;
                };
                let Some(target_lock_node) = lock.nodes.get(&lock_target_name) else {
                    draft.diagnostics.push(GraphDiagnostic::error(
                        "input_target_missing",
                        format!(
                            "node '{node_id}': input '{input_name}' targets '{lock_target_name}' not found in lock"
                        ),
                        vec![node_id.clone()],
                    ));
                    continue;
                };
                if let Some(workspace_target_id) =
                    map_lock_target_to_workspace(&lock_target_name, target_lock_node, &aliases)
                {
                    if workspace_target_id != *node_id {
                        draft.edges.push(EdgeSpec {
                            from: node_id.clone(),
                            to: workspace_target_id,
                            kind: EdgeKind::FlakeInput {
                                input_name: input_name.clone(),
                                lock_file: lock_path.clone(),
                            },
                        });
                    }
                } else {
                    draft.external_inputs.push(external_from_lock(
                        node_id.clone(),
                        input_name.clone(),
                        target_lock_node,
                    ));
                }
            }
        }

        draft.dedup_edges();
        Ok(draft)
    }
}

impl DagGenerationStrategy for FlakeLocksStrategy {
    fn name(&self) -> &'static str {
        "flake-locks"
    }

    fn generate(&self, ctx: &GenerationContext) -> Result<WorkspaceGraphDraft, StrategyError> {
        let discovery = discover_inventory(&ctx.root, ctx.metadata.as_deref())
            .map_err(|error| StrategyError::new(self.name(), error))?;
        self.generate_from_discovery(discovery)
    }
}

#[derive(Debug, Default)]
pub struct GitSubmodulesStrategy;

impl DagGenerationStrategy for GitSubmodulesStrategy {
    fn name(&self) -> &'static str {
        "git-submodules"
    }

    fn generate(&self, ctx: &GenerationContext) -> Result<WorkspaceGraphDraft, StrategyError> {
        let discovery = discover_inventory(&ctx.root, ctx.metadata.as_deref())
            .map_err(|e| StrategyError::new(self.name(), e))?;
        let root_id = discovery
            .nodes
            .values()
            .find(|n| n.is_root)
            .map(|n| n.id.clone())
            .unwrap_or_else(|| "phenix".to_string());
        let mut draft = WorkspaceGraphDraft::new(discovery.nodes.into_iter().collect());
        for node in draft.nodes.values() {
            if node.id == root_id || node.kind == NodeKind::WorkspaceRoot {
                continue;
            }
            draft.edges.push(EdgeSpec {
                from: root_id.clone(),
                to: node.id.clone(),
                kind: EdgeKind::SubmoduleMembership {
                    path: node.path.clone(),
                    gitlink: read_gitlink(&ctx.root, &node.path),
                },
            });
        }
        draft.dedup_edges();
        Ok(draft)
    }
}

fn read_gitlink(root: &std::path::Path, path: &std::path::Path) -> Option<String> {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let output = std::process::Command::new("git")
        .args(["ls-tree", "HEAD", rel.to_string_lossy().as_ref()])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.split_whitespace().nth(2).map(str::to_string)
}

#[derive(Default)]
pub struct CompositeDagGenerationStrategy {
    strategies: Vec<Box<dyn DagGenerationStrategy>>,
}

impl CompositeDagGenerationStrategy {
    pub fn new(strategies: Vec<Box<dyn DagGenerationStrategy>>) -> Self {
        Self { strategies }
    }

    pub fn default_workspace() -> Self {
        Self::new(vec![Box::new(FlakeLocksStrategy)])
    }
}

impl std::fmt::Debug for CompositeDagGenerationStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositeDagGenerationStrategy")
            .field("strategy_count", &self.strategies.len())
            .finish()
    }
}

impl DagGenerationStrategy for CompositeDagGenerationStrategy {
    fn name(&self) -> &'static str {
        "composite"
    }

    fn generate(&self, ctx: &GenerationContext) -> Result<WorkspaceGraphDraft, StrategyError> {
        let mut merged = WorkspaceGraphDraft::default();
        for strategy in &self.strategies {
            merged.merge(strategy.generate(ctx)?);
        }
        Ok(merged)
    }
}

pub fn default_generation_context(
    root: impl Into<PathBuf>,
    metadata: Option<PathBuf>,
) -> GenerationContext {
    GenerationContext {
        root: root.into(),
        metadata,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composite_dedups_strategy_edges() {
        let mut draft = WorkspaceGraphDraft::default();
        draft.edges.push(EdgeSpec {
            from: "a".into(),
            to: "b".into(),
            kind: EdgeKind::FlakeInput {
                input_name: "b-input".into(),
                lock_file: "flake.lock".into(),
            },
        });
        draft.edges.push(EdgeSpec {
            from: "a".into(),
            to: "b".into(),
            kind: EdgeKind::FlakeInput {
                input_name: "b-input".into(),
                lock_file: "flake.lock".into(),
            },
        });
        draft.dedup_edges();
        assert_eq!(draft.edges.len(), 1);
    }
}
