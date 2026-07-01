# Task request checkpoint

- task_id: stitch-dag-generation-strategy-2026-07-01
- repo: /home/matthisk/phenix/flakes/02-producers/phenix-stitch
- requested role: phenix-planner
- classification: change / c4 / high risk
- planner lease: read repo, write .phenix-agent-state only; no tracked source edits; no commit/push

## Original request
Refactor Stitch DAG construction so there is a clean typed DAG generation strategy layer:
DagGenerationStrategy -> WorkspaceGraphDraft -> CanonicalWorkspaceGraph -> DagValidator -> DagPlanner -> ExecutionPlan/StepProgram/recipes. Add flake-locks, git-submodules, composite strategies; typed NodeSpec/EdgeSpec; canonical StableDiGraph graph; planner over CanonicalWorkspaceGraph; route exec and MCP/CLI DAG inspection through graph/planner where feasible; preserve sync via adapter if needed; nix.updateInputs receives exact input names; add tests and docs.

## Non-goals
- No full interactive runner.
- No full sync-commit recipe migration unless tiny/safe.
- No generic script/plugin DAG strategy.
- No MCP-owned graph semantics.
- No commits/pushes.
