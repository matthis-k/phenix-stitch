# Stitch DAG generation strategy

Stitch now separates DAG construction into focused layers:

1. `DagGenerationStrategy` discovers graph facts only.
2. `WorkspaceGraphDraft` merges and de-duplicates typed node/edge facts.
3. `CanonicalWorkspaceGraph` stores the validated graph in a petgraph
   `StableDiGraph<NodeSpec, EdgeSpec>` with deterministic node-id maps.
4. `DagValidator` preserves topology validation rules over the canonical graph.
5. `DagPlanner` owns closure and ordering projections for execution, CLI, and MCP.

Semantic dependency edges remain **consumer -> provider**. Provider-first order is a
planner projection over those edges; strategies do not invert the edge direction.

Strategies are deliberately limited. `FlakeLocksStrategy` emits semantic flake-input
edges with the exact lock input name. `GitSubmodulesStrategy` emits membership facts
only and does not infer dependency semantics. `CompositeDagGenerationStrategy` merges
strategy output without becoming a plugin or execution system.

Execution, sync, CLI, and MCP stay thin consumers of core graph/planner/render APIs.
Sync remains a compatibility adapter in this slice: `ActionPlan` behavior is preserved,
and nix input updates use the exact `EdgeSpec`/flake input name instead of repo-name
heuristics when graph metadata is available.

Non-goals for this refactor: no full interactive runner, no full sync-commit recipe
migration, no generic script/plugin DAG source, and no MCP-owned graph semantics.
