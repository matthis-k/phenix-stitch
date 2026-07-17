use phenix_mcp_core::mcp::{McpTool, ToolContext};
use phenix_mcp_core::result::{ErrorKind, ToolFailure, ToolResult};
use phenix_mcp_core::types::{MutationLevel, ToolMetadata};
use serde_json::{json, Value};
use stitch::exec::ExecutionScope;

fn metadata() -> ToolMetadata {
    ToolMetadata {
        mutation: MutationLevel::ReadOnly,
        requires_plan: None,
        requires_clean_worktree: None,
        requires_confirmation: None,
        allowed_roots_only: Some(true),
    }
}

fn failure(message: impl Into<String>, audit_id: &str) -> ToolFailure {
    ToolFailure::new(ErrorKind::Internal, message, audit_id)
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
        metadata()
    }
    fn input_schema(&self) -> Value {
        json!({ "dirty_only": { "type": "boolean" } })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let dirty_only = input
            .get("dirty_only")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let config = stitch::config::find_and_load().map_err(|error| failure(error, &audit_id))?;
        let statuses = stitch::status::collect_all(&config)
            .map_err(|error| failure(error, &audit_id))?
            .into_iter()
            .filter(|status| !dirty_only || status.is_dirty)
            .collect::<Vec<_>>();
        Ok(serde_json::to_value(ToolResult::ok(
            json!(statuses),
            "workspace status",
            &audit_id,
        ))
        .unwrap())
    }
}

pub struct StitchGraphTool;
impl McpTool for StitchGraphTool {
    fn name(&self) -> &str {
        "stitch.graph"
    }
    fn description(&self) -> &str {
        "Return the canonical repository dependency graph"
    }
    fn metadata(&self) -> ToolMetadata {
        metadata()
    }
    fn input_schema(&self) -> Value {
        json!({})
    }
    fn call(&self, _input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let config = stitch::config::find_and_load().map_err(|error| failure(error, &audit_id))?;
        let graph = stitch::graph::derive_workspace_graph_from_config(&config, None)
            .map_err(|error| failure(error.to_string(), &audit_id))?;
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
        Ok(serde_json::to_value(ToolResult::ok(
            json!({ "nodes": nodes, "edges": edges }),
            "workspace graph",
            &audit_id,
        ))
        .unwrap())
    }
}

pub struct StitchPlanTool;
impl McpTool for StitchPlanTool {
    fn name(&self) -> &str {
        "stitch.plan"
    }
    fn description(&self) -> &str {
        "Plan a repository selection, closure, and execution order without executing"
    }
    fn metadata(&self) -> ToolMetadata {
        metadata()
    }
    fn input_schema(&self) -> Value {
        json!({
            "selection": { "type": "string", "enum": ["all", "changed", "dirty", "current", "explicit"] },
            "nodes": { "type": "array", "items": { "type": "string" } },
            "closure": { "type": "string", "enum": ["self", "upstream", "downstream", "connected", "all"] },
            "order": { "type": "string", "enum": ["stable", "providers-first", "consumers-first"] }
        })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let selection_text = input
            .get("selection")
            .and_then(Value::as_str)
            .unwrap_or("current");
        let selection = stitch::exec::parse_selection_mode(selection_text)
            .map_err(|error| failure(error, &audit_id))?;
        let nodes = input
            .get("nodes")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let closure = stitch::exec::parse_closure_mode(
            input
                .get("closure")
                .and_then(Value::as_str)
                .unwrap_or("self"),
        )
        .map_err(|error| failure(error, &audit_id))?;
        let order = stitch::exec::parse_order_mode(
            input
                .get("order")
                .and_then(Value::as_str)
                .unwrap_or("providers-first"),
        )
        .map_err(|error| failure(error, &audit_id))?;
        let config = stitch::config::find_and_load().map_err(|error| failure(error, &audit_id))?;
        let plan = stitch::exec::build_scope(
            &config,
            &ExecutionScope {
                selection,
                explicit_nodes: nodes,
                closure,
                order,
            },
        )
        .map_err(|error| failure(error, &audit_id))?;
        Ok(serde_json::to_value(ToolResult::ok(
            json!(plan),
            "ordered repository plan",
            &audit_id,
        ))
        .unwrap())
    }
}
