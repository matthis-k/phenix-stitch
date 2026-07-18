use std::path::PathBuf;

use phenix_mcp_core::mcp::{McpTool, ToolContext};
use phenix_mcp_core::result::{ErrorKind, ToolFailure, ToolResult};
use phenix_mcp_core::types::{MutationLevel, ToolMetadata};
use serde_json::{json, Value};
use stitch::exec::{ClosureMode, ExecutionScope, RunOptions, SelectionMode};
use stitch::model::WorkspaceConfig;

fn read_only_metadata() -> ToolMetadata {
    ToolMetadata {
        mutation: MutationLevel::ReadOnly,
        requires_plan: None,
        requires_clean_worktree: None,
        requires_confirmation: None,
        allowed_roots_only: Some(true),
    }
}

fn arbitrary_exec_metadata() -> ToolMetadata {
    ToolMetadata {
        mutation: MutationLevel::Arbitrary,
        requires_plan: None,
        requires_clean_worktree: None,
        requires_confirmation: None,
        allowed_roots_only: Some(true),
    }
}

fn failure(kind: ErrorKind, message: impl Into<String>, audit_id: &str) -> ToolFailure {
    ToolFailure::new(kind, message, audit_id)
}

fn success(data: Value, summary: impl Into<String>, audit_id: &str) -> Value {
    serde_json::to_value(ToolResult::ok(data, summary, audit_id))
        .expect("ToolResult must serialize")
}

fn workspace_root(input: &Value) -> PathBuf {
    input
        .get("workspace_root")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn load_config(input: &Value) -> Result<WorkspaceConfig, String> {
    match input
        .get("workspace_root")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        Some(root) => stitch::config::load_workspace_root(PathBuf::from(root).as_path()),
        None => stitch::config::find_and_load(),
    }
}

fn string_array(input: &Value, key: &str) -> Result<Vec<String>, String> {
    let Some(values) = input.get(key) else {
        return Ok(Vec::new());
    };
    let values = values
        .as_array()
        .ok_or_else(|| format!("'{key}' must be an array of strings"))?;
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("'{key}' must contain only strings"))
        })
        .collect()
}

fn execution_scope(input: &Value) -> Result<ExecutionScope, String> {
    let selection = stitch::exec::parse_selection_mode(
        input
            .get("selection")
            .and_then(Value::as_str)
            .unwrap_or("current"),
    )?;
    let explicit_nodes = string_array(input, "nodes")?;
    let closure = stitch::exec::parse_closure_mode(
        input
            .get("closure")
            .and_then(Value::as_str)
            .unwrap_or("self"),
    )?;
    let order = stitch::exec::parse_order_mode(
        input
            .get("order")
            .and_then(Value::as_str)
            .unwrap_or("providers-first"),
    )?;
    Ok(ExecutionScope {
        selection,
        explicit_nodes,
        closure,
        order,
    })
}

fn workspace_root_property() -> Value {
    json!({
        "type": "string",
        "description": "Workspace root. Defaults to discovery from the MCP server working directory."
    })
}

pub struct StitchWorkspaceDiscoverTool;
impl McpTool for StitchWorkspaceDiscoverTool {
    fn name(&self) -> &str {
        "stitch.workspace.discover"
    }

    fn description(&self) -> &str {
        "Discover the repositories in a Stitch workspace"
    }

    fn metadata(&self) -> ToolMetadata {
        read_only_metadata()
    }

    fn input_schema(&self) -> Value {
        json!({ "workspace_root": workspace_root_property() })
    }

    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let root = workspace_root(&input);
        let config = stitch::config::load_workspace_root(&root)
            .map_err(|error| failure(ErrorKind::InvalidInput, error, &audit_id))?;
        Ok(success(json!(config), "workspace discovery", &audit_id))
    }
}

pub struct StitchWorkspaceInventoryTool;
impl McpTool for StitchWorkspaceInventoryTool {
    fn name(&self) -> &str {
        "stitch.workspace.inventory"
    }

    fn description(&self) -> &str {
        "Return the read-only desired repository inventory from the root lock graph"
    }

    fn metadata(&self) -> ToolMetadata {
        read_only_metadata()
    }

    fn input_schema(&self) -> Value {
        json!({ "workspace_root": workspace_root_property() })
    }

    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let root = workspace_root(&input);
        let repositories = stitch::locked_workspace_inventory(&root)
            .map_err(|error| failure(ErrorKind::InvalidInput, error, &audit_id))?;
        Ok(success(
            json!(repositories),
            "workspace inventory",
            &audit_id,
        ))
    }
}

pub struct StitchGraphDeriveTool;
impl McpTool for StitchGraphDeriveTool {
    fn name(&self) -> &str {
        "stitch.graph.derive"
    }

    fn description(&self) -> &str {
        "Derive the canonical repository dependency graph"
    }

    fn metadata(&self) -> ToolMetadata {
        read_only_metadata()
    }

    fn input_schema(&self) -> Value {
        json!({ "workspace_root": workspace_root_property() })
    }

    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let root = workspace_root(&input);
        let config = stitch::config::load_workspace_root(&root)
            .map_err(|error| failure(ErrorKind::InvalidInput, error, &audit_id))?;
        let graph = stitch::graph::derive_workspace_graph_from_config(&config, None)
            .map_err(|error| failure(ErrorKind::InvalidInput, error.to_string(), &audit_id))?;
        let nodes = graph
            .node_ids()
            .filter_map(|id| graph.node(id))
            .cloned()
            .collect::<Vec<_>>();
        let edges = graph
            .semantic_edges()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        Ok(success(
            json!({ "nodes": nodes, "edges": edges }),
            "workspace graph",
            &audit_id,
        ))
    }
}

pub struct StitchGraphVerifyTool;
impl McpTool for StitchGraphVerifyTool {
    fn name(&self) -> &str {
        "stitch.graph.verify"
    }

    fn description(&self) -> &str {
        "Validate the canonical repository dependency graph"
    }

    fn metadata(&self) -> ToolMetadata {
        read_only_metadata()
    }

    fn input_schema(&self) -> Value {
        json!({
            "workspace_root": workspace_root_property(),
            "strict": {
                "type": "boolean",
                "description": "Treat strict graph diagnostics as validation failures."
            }
        })
    }

    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let root = workspace_root(&input);
        let strict = input
            .get("strict")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let config = stitch::config::load_workspace_root(&root)
            .map_err(|error| failure(ErrorKind::InvalidInput, error, &audit_id))?;
        let graph = stitch::graph::derive_workspace_graph_from_config(&config, None)
            .map_err(|error| failure(ErrorKind::InvalidInput, error.to_string(), &audit_id))?;
        let report =
            stitch::graph::validate_graph(&graph, &stitch::graph::ValidateOptions { strict });
        let summary = if report.valid {
            "graph validation passed"
        } else {
            "graph validation failed"
        };
        Ok(success(json!(report), summary, &audit_id))
    }
}

pub struct StitchGraphOrderTool;
impl McpTool for StitchGraphOrderTool {
    fn name(&self) -> &str {
        "stitch.graph.order"
    }

    fn description(&self) -> &str {
        "Return all repositories in the requested deterministic graph order"
    }

    fn metadata(&self) -> ToolMetadata {
        read_only_metadata()
    }

    fn input_schema(&self) -> Value {
        json!({
            "workspace_root": workspace_root_property(),
            "order": {
                "type": "string",
                "enum": ["stable", "providers-first", "consumers-first"],
                "description": "Execution order. Defaults to providers-first."
            }
        })
    }

    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let root = workspace_root(&input);
        let config = stitch::config::load_workspace_root(&root)
            .map_err(|error| failure(ErrorKind::InvalidInput, error, &audit_id))?;
        let order = stitch::exec::parse_order_mode(
            input
                .get("order")
                .and_then(Value::as_str)
                .unwrap_or("providers-first"),
        )
        .map_err(|error| failure(ErrorKind::InvalidInput, error, &audit_id))?;
        let nodes = stitch::exec::build_scope(
            &config,
            &ExecutionScope {
                selection: SelectionMode::All,
                explicit_nodes: Vec::new(),
                closure: ClosureMode::All,
                order,
            },
        )
        .map_err(|error| failure(ErrorKind::InvalidInput, error, &audit_id))?;
        Ok(success(json!(nodes), "ordered repository graph", &audit_id))
    }
}

pub struct StitchStatusTool;
impl McpTool for StitchStatusTool {
    fn name(&self) -> &str {
        "stitch.status"
    }

    fn description(&self) -> &str {
        "Return status for every discovered repository"
    }

    fn metadata(&self) -> ToolMetadata {
        read_only_metadata()
    }

    fn input_schema(&self) -> Value {
        json!({
            "workspace_root": workspace_root_property(),
            "dirty_only": {
                "type": "boolean",
                "description": "Return only repositories with worktree changes."
            }
        })
    }

    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let dirty_only = input
            .get("dirty_only")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let config = load_config(&input)
            .map_err(|error| failure(ErrorKind::InvalidInput, error, &audit_id))?;
        let statuses = stitch::status::collect_all(&config)
            .map_err(|error| failure(ErrorKind::Internal, error, &audit_id))?
            .into_iter()
            .filter(|status| !dirty_only || status.is_dirty)
            .collect::<Vec<_>>();
        Ok(success(json!(statuses), "workspace status", &audit_id))
    }
}

pub struct StitchExecTool;
impl McpTool for StitchExecTool {
    fn name(&self) -> &str {
        "stitch.exec"
    }

    fn description(&self) -> &str {
        "Execute an arbitrary argv vector over a selected repository closure in deterministic order"
    }

    fn metadata(&self) -> ToolMetadata {
        arbitrary_exec_metadata()
    }

    fn input_schema(&self) -> Value {
        json!({
            "workspace_root": workspace_root_property(),
            "selection": {
                "type": "string",
                "enum": ["all", "changed", "dirty", "current", "explicit"],
                "description": "Repository selection. Defaults to current."
            },
            "nodes": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Repository names used with explicit selection."
            },
            "closure": {
                "type": "string",
                "enum": ["self", "upstream", "downstream", "connected", "all"],
                "description": "Graph closure. Defaults to self."
            },
            "order": {
                "type": "string",
                "enum": ["stable", "providers-first", "consumers-first"],
                "description": "Execution order. Defaults to providers-first."
            },
            "dry_run": {
                "type": "boolean",
                "description": "Return the selected repositories and command without executing it."
            },
            "keep_going": {
                "type": "boolean",
                "description": "Continue after a repository command fails."
            },
            "command": {
                "type": "array",
                "items": { "type": "string" },
                "minItems": 1,
                "description": "Command argv; the first item is the executable."
            },
            "required": ["command"]
        })
    }

    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let config = load_config(&input)
            .map_err(|error| failure(ErrorKind::InvalidInput, error, &audit_id))?;
        let scope = execution_scope(&input)
            .map_err(|error| failure(ErrorKind::InvalidInput, error, &audit_id))?;
        let command = string_array(&input, "command")
            .map_err(|error| failure(ErrorKind::InvalidInput, error, &audit_id))?;
        let plan = stitch::exec::build_plan(&config, &scope, command.clone())
            .map_err(|error| failure(ErrorKind::InvalidInput, error, &audit_id))?;
        let report = stitch::exec::run_plan(
            &plan,
            &RunOptions {
                dry_run: input
                    .get("dry_run")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                keep_going: input
                    .get("keep_going")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            },
        )
        .map_err(|error| failure(ErrorKind::Internal, error, &audit_id))?;

        if report.failed_nodes > 0 {
            let failed = report
                .node_results
                .iter()
                .find(|result| !result.success)
                .expect("failed report must contain a failed node");
            let failed_nodes = report
                .node_results
                .iter()
                .filter(|result| !result.success)
                .map(|result| result.node.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let mut error = failure(
                ErrorKind::CommandFailed,
                format!(
                    "{} repository command(s) failed: {failed_nodes}",
                    report.failed_nodes
                ),
                &audit_id,
            )
            .with_command(command)
            .with_stdout(&failed.stdout)
            .with_stderr(&failed.stderr);
            if let Some(exit_code) = failed.exit_code {
                error = error.with_exit_code(exit_code);
            }
            return Err(error);
        }

        Ok(success(json!(report), "repository execution", &audit_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_mirror_the_cli_surface() {
        assert_eq!(
            StitchWorkspaceDiscoverTool.name(),
            "stitch.workspace.discover"
        );
        assert_eq!(
            StitchWorkspaceInventoryTool.name(),
            "stitch.workspace.inventory"
        );
        assert_eq!(StitchGraphDeriveTool.name(), "stitch.graph.derive");
        assert_eq!(StitchGraphVerifyTool.name(), "stitch.graph.verify");
        assert_eq!(StitchGraphOrderTool.name(), "stitch.graph.order");
        assert_eq!(StitchStatusTool.name(), "stitch.status");
        assert_eq!(StitchExecTool.name(), "stitch.exec");
    }

    #[test]
    fn exec_is_explicitly_arbitrary_without_apply_schema() {
        let tool = StitchExecTool;
        let schema = tool.input_schema();

        assert_eq!(tool.metadata().mutation, MutationLevel::Arbitrary);
        assert!(schema.get("apply").is_none());
        assert_eq!(schema.get("required"), Some(&json!(["command"])));
    }
}
