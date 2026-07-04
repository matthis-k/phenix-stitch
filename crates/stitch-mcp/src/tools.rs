use std::collections::BTreeMap;

use phenix_mcp_core::mcp::{McpTool, ToolContext};
use phenix_mcp_core::result::{ErrorKind, ToolFailure, ToolResult};
use phenix_mcp_core::types::{MutationLevel, ToolMetadata};
use serde_json::{json, Value};

use stitch::exec;
use stitch::workloop;
use stitch::workloop::LoopBackend;

fn mk_err(kind: ErrorKind, msg: &str, audit_id: &str) -> ToolFailure {
    ToolFailure::new(kind, msg, audit_id)
}

fn tool_meta(mutation: MutationLevel) -> ToolMetadata {
    ToolMetadata {
        mutation,
        requires_plan: None,
        requires_clean_worktree: None,
        requires_confirmation: None,
        allowed_roots_only: Some(true),
    }
}

fn run_exec_plan(
    cfg: &stitch::model::WorkspaceConfig,
    selection: exec::SelectionMode,
    explicit_nodes: Vec<String>,
    closure: exec::ClosureMode,
    order: exec::OrderMode,
    step: exec::ExecutionStep,
) -> Result<exec::ExecutionReport, String> {
    let scope = exec::ExecutionScope {
        selection,
        explicit_nodes,
        closure,
        order,
    };
    let plan = exec::build_plan(cfg, &scope, vec![step])?;
    let opts = capture_exec_run_options();
    exec::run_plan(cfg, &plan, &opts)
}

fn capture_exec_run_options() -> exec::RunOptions {
    exec::RunOptions {
        dry_run: false,
        apply: false,
        json: true,
    }
}

fn collect_status_json(
    cfg: &stitch::model::WorkspaceConfig,
    repo_filter: &[String],
) -> Result<Vec<Value>, String> {
    let selection = if repo_filter.is_empty() {
        exec::SelectionMode::All
    } else {
        exec::SelectionMode::Explicit
    };
    let step = exec::ExecutionStep {
        id: "collect-status".to_string(),
        mode: exec::ExecutionMode::ReadOnly,
        kind: exec::StepKind::Builtin {
            name: "git.collect-status".to_string(),
            args: serde_json::Value::Null,
        },
        condition: None,
    };
    let report = run_exec_plan(
        cfg,
        selection,
        repo_filter.to_vec(),
        exec::ClosureMode::SelfOnly,
        exec::OrderMode::Stable,
        step,
    )?;
    let mut statuses = Vec::new();
    for nr in &report.node_results {
        for sr in &nr.step_results {
            if sr.success && !sr.stdout.is_empty() {
                if let Ok(val) = serde_json::from_str::<Value>(&sr.stdout) {
                    statuses.push(val);
                }
            }
        }
    }
    Ok(statuses)
}

pub struct StitchStatusTool;

impl McpTool for StitchStatusTool {
    fn name(&self) -> &str {
        "stitch.status"
    }
    fn description(&self) -> &str {
        "Show multi-repo git status across all configured repos"
    }
    fn metadata(&self) -> ToolMetadata {
        tool_meta(MutationLevel::ReadOnly)
    }
    fn input_schema(&self) -> Value {
        json!({
            "repos": { "type": "array", "items": {"type": "string"} },
            "include_untracked": { "type": "boolean" },
            "include_remote": { "type": "boolean" },
            "short": { "type": "boolean" },
            "dirty_only": { "type": "boolean" }
        })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let repo_filter: Vec<String> = input
            .get("repos")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();
        let dirty_only = input
            .get("dirty_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let short = input
            .get("short")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let include_untracked = input
            .get("include_untracked")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let cfg = match stitch::config::find_and_load() {
            Ok(c) => c,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::NotFound,
                    &format!("Config: {}", e),
                    &audit_id,
                ))
            }
        };

        let statuses = collect_status_json(&cfg, &repo_filter).map_err(|e| {
            mk_err(
                ErrorKind::Internal,
                &format!("Status collection failed: {e}"),
                &audit_id,
            )
        })?;
        let mut repo_statuses: Vec<Value> = Vec::new();
        let mut short_lines: Vec<String> = Vec::new();
        let mut dirty_repos: Vec<String> = Vec::new();

        for s in &statuses {
            let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let is_dirty = s.get("is_dirty").and_then(|v| v.as_bool()).unwrap_or(false);
            if dirty_only && !is_dirty {
                continue;
            }
            if is_dirty {
                dirty_repos.push(name.to_string());
            }

            let changes: Vec<String> = if include_untracked {
                let repo_cfg = cfg.repos.iter().find(|r| r.name == name);
                repo_cfg
                    .map(|r| {
                        stitch::git::git_diff_names(&r.resolved_path(&cfg)).unwrap_or_default()
                    })
                    .unwrap_or_default()
            } else {
                vec![]
            };

            if short {
                for f in &changes {
                    short_lines.push(format!("M  {}", f));
                }
            }

            let untracked_count = if include_untracked {
                s.get("untracked_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
            } else {
                0
            };
            repo_statuses.push(json!({
                "name": s.get("name"),
                "path": s.get("path"),
                "branch": s.get("branch"),
                "head": "",
                "dirty": is_dirty,
                "staged_count": s.get("staged_count"),
                "unstaged_count": s.get("unstaged_count"),
                "untracked_count": untracked_count,
                "changes": changes.iter().map(|f| json!({"status": "modified", "path": f})).collect::<Vec<_>>()
            }));
        }

        let result = ToolResult::ok(
            json!({
                "workspace": cfg.workspace,
                "repos": repo_statuses,
                "dirty_repos": dirty_repos,
                "short": short_lines,
                "total": repo_statuses.len()
            }),
            format!("{} repos, {} dirty", repo_statuses.len(), dirty_repos.len()),
            &audit_id,
        );
        Ok(serde_json::to_value(&result).unwrap_or_default())
    }
}

pub struct StitchDiffTool;

impl McpTool for StitchDiffTool {
    fn name(&self) -> &str {
        "stitch.diff"
    }
    fn description(&self) -> &str {
        "Show diffs across repos"
    }
    fn metadata(&self) -> ToolMetadata {
        tool_meta(MutationLevel::ReadOnly)
    }
    fn input_schema(&self) -> Value {
        json!({
            "repo": { "type": "string", "description": "Repo name" },
            "staged": { "type": "boolean" },
            "json": { "type": "boolean" }
        })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let repo_name = input
            .get("repo")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let staged = input
            .get("staged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let cfg = match stitch::config::find_and_load() {
            Ok(c) => c,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::NotFound,
                    &format!("Config: {}", e),
                    &audit_id,
                ))
            }
        };

        let explicit_nodes: Vec<String> = if repo_name.is_empty() {
            Vec::new()
        } else {
            vec![repo_name.clone()]
        };
        let selection = if repo_name.is_empty() {
            exec::SelectionMode::All
        } else {
            exec::SelectionMode::Explicit
        };

        let step = exec::ExecutionStep {
            id: "git-diff".to_string(),
            mode: exec::ExecutionMode::ReadOnly,
            kind: exec::StepKind::Builtin {
                name: "git.diff".to_string(),
                args: json!({"staged": staged}),
            },
            condition: None,
        };

        let report = match run_exec_plan(
            &cfg,
            selection,
            explicit_nodes,
            exec::ClosureMode::SelfOnly,
            exec::OrderMode::Stable,
            step,
        ) {
            Ok(r) => r,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Diff failed: {e}"),
                    &audit_id,
                ))
            }
        };

        let mut diffs: Vec<Value> = Vec::new();
        for nr in &report.node_results {
            for sr in &nr.step_results {
                if !sr.success {
                    continue;
                }
                let diff_text = sr.stdout.trim().to_string();
                let files: Vec<String> = if !staged && !diff_text.is_empty() {
                    diff_text
                        .lines()
                        .filter_map(|l| {
                            if l.starts_with("diff --git")
                                || l.starts_with("--- ")
                                || l.starts_with("+++ ")
                                || l.starts_with("@@")
                            {
                                None
                            } else {
                                Some(l.to_string())
                            }
                        })
                        .collect()
                } else {
                    vec![]
                };
                let mut entry = json!({
                    "repo": nr.node,
                    "diff": diff_text,
                });
                if !files.is_empty() {
                    entry["files"] = json!(files);
                }
                diffs.push(entry);
            }
        }

        let result = ToolResult::ok(
            json!({ "diffs": diffs, "total": diffs.len() }),
            format!("{} diff(s)", diffs.len()),
            &audit_id,
        );
        Ok(serde_json::to_value(&result).unwrap_or_default())
    }
}

pub struct StitchDagTool;

impl McpTool for StitchDagTool {
    fn name(&self) -> &str {
        "stitch.dag"
    }
    fn description(&self) -> &str {
        "Show ordered operation DAG for commit or sync (read-only)"
    }
    fn metadata(&self) -> ToolMetadata {
        tool_meta(MutationLevel::ReadOnly)
    }
    fn input_schema(&self) -> Value {
        json!({
            "mode": { "type": "string", "enum": ["commit", "sync", "full"] },
            "repos": { "type": "array", "items": {"type": "string"} },
            "staged": { "type": "boolean", "description": "Use staged files only" },
            "split": { "type": "string", "enum": ["by-repo", "by-path", "manual"] },
            "run_tend": { "type": "boolean" },
            "json": { "type": "boolean" }
        })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let mode = input
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("commit");
        let split = input
            .get("split")
            .and_then(|v| v.as_str())
            .unwrap_or("by-repo");
        let staged = input
            .get("staged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let run_tend = input
            .get("run_tend")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let repo_filter: Vec<String> = input
            .get("repos")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();

        let cfg = match stitch::config::find_and_load() {
            Ok(c) => c,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::NotFound,
                    &format!("Config: {}", e),
                    &audit_id,
                ))
            }
        };

        let dag = stitch::graph::render::operation_dag_json(
            &cfg,
            mode,
            split,
            staged,
            run_tend,
            &repo_filter,
        )
        .map_err(|e| {
            mk_err(
                ErrorKind::Internal,
                &format!("DAG rendering failed: {e}"),
                &audit_id,
            )
        })?;
        let total = dag.get("total").and_then(|v| v.as_u64()).unwrap_or(0);

        let result = ToolResult::ok(
            dag,
            format!("DAG: {} node(s) in {} mode", total, mode),
            &audit_id,
        );
        Ok(serde_json::to_value(&result).unwrap_or_default())
    }
}

pub struct StitchCommitTemplateTool;

impl McpTool for StitchCommitTemplateTool {
    fn name(&self) -> &str {
        "stitch.commit_template"
    }
    fn description(&self) -> &str {
        "Generate a JSON message template for sync commit nodes (read-only)"
    }
    fn metadata(&self) -> ToolMetadata {
        tool_meta(MutationLevel::ReadOnly)
    }
    fn input_schema(&self) -> Value {
        json!({
            "dag_id": { "type": "string", "description": "DAG ID from stitch.dag" },
            "repos": { "type": "array", "items": {"type": "string"} },
            "staged": { "type": "boolean", "description": "Use staged files only" }
        })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let _dag_id = input.get("dag_id").and_then(|v| v.as_str());
        let staged = input
            .get("staged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let cfg = match stitch::config::find_and_load() {
            Ok(c) => c,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::NotFound,
                    &format!("Config: {}", e),
                    &audit_id,
                ))
            }
        };

        let statuses = collect_status_json(&cfg, &[]).map_err(|e| {
            mk_err(
                ErrorKind::Internal,
                &format!("Template status discovery failed: {e}"),
                &audit_id,
            )
        })?;
        let mut messages = serde_json::Map::new();

        for s in &statuses {
            let name = s.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let is_dirty = s.get("is_dirty").and_then(|v| v.as_bool()).unwrap_or(false);
            if !is_dirty {
                continue;
            }

            let repo_cfg = match cfg.repos.iter().find(|r| r.name == name) {
                Some(r) => r,
                None => continue,
            };
            let repo_path = repo_cfg.resolved_path(&cfg);
            let diff = if staged {
                stitch::git::git_diff_cached_names(&repo_path).unwrap_or_default()
            } else {
                stitch::git::git_diff_names(&repo_path).unwrap_or_default()
            };

            messages.insert(
                format!("{}:commit", name),
                json!({ "subject": "", "body": "", "files": diff }),
            );
        }

        let result = ToolResult::ok(
            json!({ "messages": messages, "total": messages.len() }),
            format!("{} commit node(s) need messages", messages.len()),
            &audit_id,
        );
        Ok(serde_json::to_value(&result).unwrap_or_default())
    }
}

pub struct StitchCommitTool;

impl McpTool for StitchCommitTool {
    fn name(&self) -> &str {
        "stitch.commit"
    }
    fn description(&self) -> &str {
        "Commit changed nodes in dependency order. Requires apply: true"
    }
    fn metadata(&self) -> ToolMetadata {
        tool_meta(MutationLevel::CreatesCommit)
    }
    fn input_schema(&self) -> Value {
        json!({
            "apply": { "type": "boolean", "description": "Must be true to execute" },
            "dry_run": { "type": "boolean", "description": "Plan mode (no mutations)" },
            "scope": { "type": "array", "items": {"type": "string"}, "description": "Repo names to commit (empty = all changed)" },
            "no_push": { "type": "boolean", "description": "Commit locally without pushing" },
            "force": { "type": "boolean", "description": "Allow edge cases like detached HEAD" },
            "messages": {
                "type": "object",
                "description": "Keyed by node name, each with subject",
                "additionalProperties": {
                    "type": "object",
                    "properties": {
                        "subject": { "type": "string" },
                        "body": { "type": "string" },
                        "files": { "type": "array", "items": { "type": "string" } }
                    }
                }
            },
            "resume": { "type": "string", "description": "Transaction ID to resume" }
        })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let apply = input
            .get("apply")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let dry_run = input
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let no_push = input
            .get("no_push")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let force = input
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let scope_repos: Vec<String> = input
            .get("scope")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();

        if !apply && !dry_run {
            return Err(mk_err(
                ErrorKind::PolicyDenied,
                "Must set apply=true to execute, or use dry_run=true for plan-only",
                &audit_id,
            ));
        }

        let cfg = match stitch::config::find_and_load() {
            Ok(c) => c,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::NotFound,
                    &format!("Config: {}", e),
                    &audit_id,
                ))
            }
        };

        let scope = if scope_repos.is_empty() {
            exec::ExecutionScope {
                selection: exec::SelectionMode::Changed,
                explicit_nodes: Vec::new(),
                closure: exec::ClosureMode::Connected,
                order: exec::OrderMode::ProvidersFirst,
            }
        } else {
            exec::ExecutionScope {
                selection: exec::SelectionMode::Explicit,
                explicit_nodes: scope_repos.clone(),
                closure: exec::ClosureMode::Connected,
                order: exec::OrderMode::ProvidersFirst,
            }
        };

        let raw_nodes = match exec::build_scope(&cfg, &scope) {
            Ok(n) => n,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Scope build failed: {}", e),
                    &audit_id,
                ))
            }
        };

        let dirty_nodes: Vec<&exec::ExecutionNode> =
            raw_nodes.iter().filter(|n| n.directly_changed).collect();
        // When scope_repos is specified, ExecutionScope::Explicit with scope_repos
        // ensures only those repos are selected; the dirty filter respects that.

        if dirty_nodes.is_empty() && dry_run {
            let out = ToolResult::ok(
                json!({"actions": [], "message": "Nothing to commit"}),
                "No changes to commit",
                &audit_id,
            );
            return Ok(serde_json::to_value(&out).unwrap_or_default());
        }

        // Load messages from input
        let messages: Option<BTreeMap<String, String>> = input
            .get("messages")
            .and_then(|v| v.as_object())
            .map(|obj| {
                obj.iter()
                    .map(|(k, v)| {
                        let msg = v.get("subject").and_then(|s| s.as_str()).unwrap_or(k);
                        (k.clone(), msg.to_string())
                    })
                    .collect()
            });

        if dirty_nodes.is_empty() {
            return Err(mk_err(
                ErrorKind::NotFound,
                "No dirty nodes to commit",
                &audit_id,
            ));
        }

        // Build per-node commit + push steps
        let mut commit_nodes: Vec<exec::ExecutionNode> = Vec::new();
        for node in dirty_nodes {
            let msg = messages
                .as_ref()
                .and_then(|m| m.get(&node.name))
                .cloned()
                .unwrap_or_default();
            if msg.is_empty() && !dry_run && !force {
                return Err(mk_err(
                    ErrorKind::InvalidInput,
                    &format!("Missing commit message for '{}'", node.name),
                    &audit_id,
                ));
            }

            let mut steps = Vec::new();
            steps.push(exec::ExecutionStep {
                id: "git-commit".to_string(),
                mode: exec::ExecutionMode::Mutating,
                kind: exec::StepKind::Builtin {
                    name: "git.commit".to_string(),
                    args: json!({"message": msg, "stage": true}),
                },
                condition: None,
            });

            if !no_push {
                steps.push(exec::ExecutionStep {
                    id: "git-push".to_string(),
                    mode: exec::ExecutionMode::Mutating,
                    kind: exec::StepKind::Builtin {
                        name: "git.push".to_string(),
                        args: serde_json::Value::Null,
                    },
                    condition: None,
                });
            }

            let mut n = node.clone();
            n.steps = steps;
            commit_nodes.push(n);
        }

        let plan = exec::ExecutionPlan {
            nodes: commit_nodes,
        };

        if dry_run {
            let action_list: Vec<Value> = plan
                .nodes
                .iter()
                .map(|n| json!({"type": "commit", "node": n.name}))
                .collect();
            let out = ToolResult::ok(
                json!({"actions": action_list, "nodes": plan.nodes.iter().map(|n| {
                    json!({"name": n.name, "directly_changed": n.directly_changed})
                }).collect::<Vec<_>>() }),
                format!("Plan: {} node(s) to commit", plan.nodes.len()),
                &audit_id,
            );
            return Ok(serde_json::to_value(&out).unwrap_or_default());
        }

        let opts = exec::RunOptions {
            dry_run: false,
            apply: true,
            json: false,
        };

        match exec::run_plan(&cfg, &plan, &opts) {
            Ok(report) => {
                let created_commits: Vec<String> = report
                    .node_results
                    .iter()
                    .filter(|nr| nr.success)
                    .map(|nr| nr.node.clone())
                    .collect();
                let push_results: Vec<Value> = report.node_results.iter()
                    .filter(|_nr| !no_push)
                    .map(|nr| {
                        json!({"node": nr.node, "success": nr.success, "error": nr.step_results.first().map(|sr| sr.stderr.clone()).filter(|e| !e.is_empty())})
                    })
                    .collect();
                let out = ToolResult::ok(
                    json!({
                        "created_commits": created_commits,
                        "push_results": push_results,
                        "total": report.total_nodes,
                        "successful": report.successful_nodes,
                        "failed": report.failed_nodes,
                    }),
                    format!(
                        "{} node(s) committed, {} succeeded",
                        report.total_nodes, report.successful_nodes
                    ),
                    &audit_id,
                );
                Ok(serde_json::to_value(&out).unwrap_or_default())
            }
            Err(e) => Err(mk_err(
                ErrorKind::Internal,
                &format!("Commit execution failed: {}", e),
                &audit_id,
            )),
        }
    }
}

pub struct StitchSyncTool;

impl McpTool for StitchSyncTool {
    fn name(&self) -> &str {
        "stitch.sync"
    }
    fn description(&self) -> &str {
        "Sync workspace: update flake inputs, run checks, and push in dependency order. Requires apply: true"
    }
    fn metadata(&self) -> ToolMetadata {
        tool_meta(MutationLevel::Network)
    }
    fn input_schema(&self) -> Value {
        json!({
            "apply": { "type": "boolean", "description": "Must be true to execute" },
            "dry_run": { "type": "boolean", "description": "Plan mode (no mutations)" },
            "scope": { "type": "array", "items": {"type": "string"}, "description": "Repo names to sync (empty = all changed)" },
            "repos": { "type": "array", "items": {"type": "string"} },
            "mode": { "type": "string", "enum": ["pull", "push", "full"] },
            "run_tend": { "type": "boolean", "description": "Run tend checks before sync" },
            "no_push": { "type": "boolean", "description": "Skip push step" }
        })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let apply = input
            .get("apply")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let dry_run = input
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let no_push = input
            .get("no_push")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let run_tend = input
            .get("run_tend")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let _mode = input.get("mode").and_then(|v| v.as_str()).unwrap_or("push");
        let repo_filter: Vec<String> = input
            .get("repos")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();

        if !apply && !dry_run {
            return Err(mk_err(
                ErrorKind::PolicyDenied,
                "Must set apply=true or dry_run=true",
                &audit_id,
            ));
        }

        let cfg = match stitch::config::find_and_load() {
            Ok(c) => c,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::NotFound,
                    &format!("Config: {}", e),
                    &audit_id,
                ))
            }
        };

        let explicit_nodes = if repo_filter.is_empty() {
            Vec::new()
        } else {
            repo_filter
        };
        let selection = if explicit_nodes.is_empty() {
            exec::SelectionMode::Changed
        } else {
            exec::SelectionMode::Explicit
        };
        let scope = exec::ExecutionScope {
            selection,
            explicit_nodes,
            closure: exec::ClosureMode::Connected,
            order: exec::OrderMode::ProvidersFirst,
        };

        let raw_nodes = match exec::build_scope(&cfg, &scope) {
            Ok(n) => n,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Scope build failed: {}", e),
                    &audit_id,
                ))
            }
        };

        let active_nodes: Vec<&exec::ExecutionNode> = raw_nodes
            .iter()
            .filter(|n| n.directly_changed || n.downstream_only)
            .collect();

        if active_nodes.is_empty() && dry_run {
            let out = ToolResult::ok(
                json!({"actions": [], "message": "Nothing to sync"}),
                "No changes to sync",
                &audit_id,
            );
            return Ok(serde_json::to_value(&out).unwrap_or_default());
        }

        // Build per-node sync steps
        let mut sync_nodes: Vec<exec::ExecutionNode> = Vec::new();
        for node in active_nodes {
            let mut steps = Vec::new();

            if node.path.join("flake.lock").exists() {
                steps.push(exec::ExecutionStep {
                    id: "update-inputs".to_string(),
                    mode: exec::ExecutionMode::Mutating,
                    kind: exec::StepKind::Builtin {
                        name: "nix.updateInputs".to_string(),
                        args: serde_json::Value::Null,
                    },
                    condition: Some(exec::StepCondition::HasLockfile),
                });
            }

            if run_tend {
                steps.push(exec::ExecutionStep {
                    id: "tend-check".to_string(),
                    mode: exec::ExecutionMode::ReadOnly,
                    kind: exec::StepKind::Builtin {
                        name: "tend.check".to_string(),
                        args: json!({"profile": "pre-push", "affected_dag": true}),
                    },
                    condition: Some(exec::StepCondition::DirectlyChanged),
                });
            }

            if !no_push {
                steps.push(exec::ExecutionStep {
                    id: "git-push".to_string(),
                    mode: exec::ExecutionMode::Mutating,
                    kind: exec::StepKind::Builtin {
                        name: "git.push".to_string(),
                        args: serde_json::Value::Null,
                    },
                    condition: Some(exec::StepCondition::DirectlyChanged),
                });
            }

            if !steps.is_empty() {
                let mut n = node.clone();
                n.steps = steps;
                sync_nodes.push(n);
            }
        }

        if sync_nodes.is_empty() && dry_run {
            let out = ToolResult::ok(
                json!({"actions": [], "message": "Nothing to sync"}),
                "No steps to execute",
                &audit_id,
            );
            return Ok(serde_json::to_value(&out).unwrap_or_default());
        }

        if sync_nodes.is_empty() {
            return Err(mk_err(
                ErrorKind::NotFound,
                "No sync steps to execute",
                &audit_id,
            ));
        }

        let plan = exec::ExecutionPlan { nodes: sync_nodes };

        if dry_run {
            let action_list: Vec<Value> = plan.nodes.iter().map(|n| {
                json!({"node": n.name, "steps": n.steps.iter().map(|s| s.id.clone()).collect::<Vec<_>>()})
            }).collect();
            let out = ToolResult::ok(
                json!({"actions": action_list, "total": plan.nodes.len()}),
                format!(
                    "Sync plan: {} node(s) with {} step(s)",
                    plan.nodes.len(),
                    plan.nodes.iter().map(|n| n.steps.len()).sum::<usize>()
                ),
                &audit_id,
            );
            return Ok(serde_json::to_value(&out).unwrap_or_default());
        }

        let opts = exec::RunOptions {
            dry_run: false,
            apply: true,
            json: false,
        };

        match exec::run_plan(&cfg, &plan, &opts) {
            Ok(report) => {
                let results: Vec<Value> = report.node_results.iter().map(|nr| {
                    json!({"name": nr.node, "success": nr.success, "error": nr.step_results.first().map(|sr| sr.stderr.clone()).filter(|e| !e.is_empty())})
                }).collect();
                let out = ToolResult::ok(
                    json!({"completed": results, "total": report.total_nodes, "successful": report.successful_nodes, "failed": report.failed_nodes}),
                    format!(
                        "{} node(s) synced, {} succeeded",
                        report.total_nodes, report.successful_nodes
                    ),
                    &audit_id,
                );
                Ok(serde_json::to_value(&out).unwrap_or_default())
            }
            Err(e) => Err(mk_err(
                ErrorKind::Internal,
                &format!("Sync execution failed: {}", e),
                &audit_id,
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Loop detection tool
// ---------------------------------------------------------------------------

pub struct StitchLoopDetectTool;

impl McpTool for StitchLoopDetectTool {
    fn name(&self) -> &str {
        "stitch.loop_detect"
    }
    fn description(&self) -> &str {
        "Detect VCS backend for a repo path"
    }
    fn metadata(&self) -> ToolMetadata {
        tool_meta(MutationLevel::ReadOnly)
    }
    fn input_schema(&self) -> Value {
        json!({
            "repo": { "type": "string", "description": "Repo path (default: current dir)" }
        })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let repo_path = input
            .get("repo")
            .and_then(|v| v.as_str())
            .map(|s| std::path::Path::new(s).to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let backend = match workloop::detect_backend(&repo_path) {
            Ok(b) => b,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Backend detection failed: {e}"),
                    &audit_id,
                ))
            }
        };
        let detection = match backend.detect(&repo_path) {
            Ok(d) => d,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Detection failed: {e}"),
                    &audit_id,
                ))
            }
        };

        let result = ToolResult::ok(
            serde_json::to_value(&detection).unwrap_or_default(),
            format!("Backend: {:?}", detection.state),
            &audit_id,
        );
        Ok(serde_json::to_value(&result).unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------
// Loop snapshot tool
// ---------------------------------------------------------------------------

pub struct StitchLoopSnapshotTool;

impl McpTool for StitchLoopSnapshotTool {
    fn name(&self) -> &str {
        "stitch.loop_snapshot"
    }
    fn description(&self) -> &str {
        "Take snapshot of current VCS state"
    }
    fn metadata(&self) -> ToolMetadata {
        tool_meta(MutationLevel::ReadOnly)
    }
    fn input_schema(&self) -> Value {
        json!({
            "repo": { "type": "string", "description": "Repo path (default: current dir)" }
        })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let repo_path = input
            .get("repo")
            .and_then(|v| v.as_str())
            .map(|s| std::path::Path::new(s).to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let repo_name = repo_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("default")
            .to_string();

        let backend = match workloop::detect_backend(&repo_path) {
            Ok(b) => b,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Backend detection failed: {e}"),
                    &audit_id,
                ))
            }
        };
        let snapshot = match backend.snapshot(&repo_path, &repo_name) {
            Ok(s) => s,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Snapshot failed: {e}"),
                    &audit_id,
                ))
            }
        };

        let result = ToolResult::ok(
            serde_json::to_value(&snapshot).unwrap_or_default(),
            format!("Snapshot: {} @ {}", snapshot.repo_name, snapshot.commit_id),
            &audit_id,
        );
        Ok(serde_json::to_value(&result).unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------
// Loop checkpoint tool
// ---------------------------------------------------------------------------

pub struct StitchLoopCheckpointTool;

impl McpTool for StitchLoopCheckpointTool {
    fn name(&self) -> &str {
        "stitch.loop_checkpoint"
    }
    fn description(&self) -> &str {
        "Create a resumable development checkpoint"
    }
    fn metadata(&self) -> ToolMetadata {
        tool_meta(MutationLevel::CreatesCommit)
    }
    fn input_schema(&self) -> Value {
        json!({
            "repo": { "type": "string", "description": "Repo path (default: current dir)" },
            "feature": { "type": "string", "description": "Feature name for the wallet" },
            "message": { "type": "string", "description": "Checkpoint message" }
        })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let repo_path = input
            .get("repo")
            .and_then(|v| v.as_str())
            .map(|s| std::path::Path::new(s).to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let feature = input
            .get("feature")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        let message = input
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("checkpoint");
        let repo_name = repo_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("default")
            .to_string();

        let cfg = match stitch::config::find_and_load() {
            Ok(c) => c,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::NotFound,
                    &format!("Config: {}", e),
                    &audit_id,
                ))
            }
        };
        let workspace_root = std::path::Path::new(&cfg.workspace);

        let backend = match workloop::detect_backend(&repo_path) {
            Ok(b) => b,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Backend detection failed: {e}"),
                    &audit_id,
                ))
            }
        };

        let checkpoint = match backend.checkpoint(&repo_path, &repo_name, message) {
            Ok(cp) => cp,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Checkpoint failed: {e}"),
                    &audit_id,
                ))
            }
        };

        // Load or create wallet
        let mut wallet = match workloop::load_wallet(workspace_root, feature) {
            Ok(w) => w,
            Err(_) => workloop::LoopWallet {
                schema_version: 1,
                loop_id: format!("loop-{}", feature),
                feature: feature.to_string(),
                backend: workloop::VcsBackend::Jj,
                state: workloop::LoopState::DirtyDev,
                repos: Vec::new(),
                verification: workloop::VerificationPointer {
                    dev_profile: None,
                    dev_status: workloop::CheckStatus::NotRun,
                    release_profile: None,
                    release_status: workloop::CheckStatus::NotRun,
                    last_evidence_id: None,
                },
                decisions: Vec::new(),
                blockers: Vec::new(),
                handoff: None,
                next_valid_actions: workloop::valid_actions_for_state(
                    &workloop::LoopState::DirtyDev,
                ),
                created_at: workloop::Timestamp::now(),
                updated_at: workloop::Timestamp::now(),
                revision: 1,
            },
        };

        // Add/update repo ref in wallet
        let repo_ref = workloop::RepoLoopRef {
            name: repo_name.clone(),
            path: repo_path.clone(),
            workspace: None,
            base_operation_id: checkpoint.operation_id.clone(),
            current_operation_id: checkpoint.operation_id.clone(),
            working_copy_change_id: checkpoint.change_id.clone(),
            working_copy_commit_id: String::new(),
            main_bookmark: "main".to_string(),
            feature_bookmark: None,
            release_candidate_change_id: None,
            exported_git_commit: None,
            release_git_commit: None,
        };
        if let Some(pos) = wallet.repos.iter().position(|r| r.name == repo_name) {
            wallet.repos[pos] = repo_ref;
        } else {
            wallet.repos.push(repo_ref);
        }
        wallet.updated_at = workloop::Timestamp::now();
        wallet.revision += 1;

        if let Err(e) = workloop::save_wallet(workspace_root, &wallet) {
            return Err(mk_err(
                ErrorKind::Internal,
                &format!("Failed to save wallet: {e}"),
                &audit_id,
            ));
        }

        let result = ToolResult::ok(
            serde_json::to_value(&checkpoint).unwrap_or_default(),
            format!("Checkpoint: {} ({})", message, checkpoint.change_id),
            &audit_id,
        );
        Ok(serde_json::to_value(&result).unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------
// Loop current-change tool
// ---------------------------------------------------------------------------

pub struct StitchLoopCurrentChangeTool;

impl McpTool for StitchLoopCurrentChangeTool {
    fn name(&self) -> &str {
        "stitch.loop_current_change"
    }
    fn description(&self) -> &str {
        "Get current change identity"
    }
    fn metadata(&self) -> ToolMetadata {
        tool_meta(MutationLevel::ReadOnly)
    }
    fn input_schema(&self) -> Value {
        json!({
            "repo": { "type": "string", "description": "Repo path (default: current dir)" }
        })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let repo_path = input
            .get("repo")
            .and_then(|v| v.as_str())
            .map(|s| std::path::Path::new(s).to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let repo_name = repo_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("default")
            .to_string();

        let backend = match workloop::detect_backend(&repo_path) {
            Ok(b) => b,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Backend detection failed: {e}"),
                    &audit_id,
                ))
            }
        };
        let change = match backend.current_change(&repo_path, &repo_name) {
            Ok(c) => c,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Current change failed: {e}"),
                    &audit_id,
                ))
            }
        };

        let result = ToolResult::ok(
            serde_json::to_value(&change).unwrap_or_default(),
            format!("Change: {} (empty: {})", change.change_id, change.is_empty),
            &audit_id,
        );
        Ok(serde_json::to_value(&result).unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------
// Loop create-release-candidate tool
// ---------------------------------------------------------------------------

pub struct StitchLoopCreateRcTool;

impl McpTool for StitchLoopCreateRcTool {
    fn name(&self) -> &str {
        "stitch.loop_create_rc"
    }
    fn description(&self) -> &str {
        "Create a release candidate"
    }
    fn metadata(&self) -> ToolMetadata {
        tool_meta(MutationLevel::CreatesCommit)
    }
    fn input_schema(&self) -> Value {
        json!({
            "repo": { "type": "string", "description": "Repo path (default: current dir)" },
            "feature": { "type": "string", "description": "Feature name for the wallet" },
            "target": { "type": "string", "description": "Target bookmark (default: main)" }
        })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let repo_path = input
            .get("repo")
            .and_then(|v| v.as_str())
            .map(|s| std::path::Path::new(s).to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let feature = input
            .get("feature")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        let target = input
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("main");
        let repo_name = repo_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("default")
            .to_string();

        let cfg = match stitch::config::find_and_load() {
            Ok(c) => c,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::NotFound,
                    &format!("Config: {}", e),
                    &audit_id,
                ))
            }
        };
        let workspace_root = std::path::Path::new(&cfg.workspace);

        // Load wallet (must exist)
        let mut wallet = match workloop::load_wallet(workspace_root, feature) {
            Ok(w) => w,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::NotFound,
                    &format!("Wallet not found: {e}"),
                    &audit_id,
                ))
            }
        };

        let backend = match workloop::detect_backend(&repo_path) {
            Ok(b) => b,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Backend detection failed: {e}"),
                    &audit_id,
                ))
            }
        };

        // Get current change as source
        let change = match backend.current_change(&repo_path, &repo_name) {
            Ok(c) => c,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Failed to get current change: {e}"),
                    &audit_id,
                ))
            }
        };

        let release_input = workloop::ReleaseInput {
            repo_name: repo_name.clone(),
            source_change_id: change.change_id.clone(),
            target_bookmark: target.to_string(),
            squash_message: None,
        };

        let rc = match backend.create_release_candidate(&repo_path, release_input) {
            Ok(rc) => rc,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("RC creation failed: {e}"),
                    &audit_id,
                ))
            }
        };

        // Update wallet
        if let Some(pos) = wallet.repos.iter().position(|r| r.name == repo_name) {
            wallet.repos[pos].release_candidate_change_id = Some(rc.change_id.clone());
        }
        wallet.state = workloop::LoopState::ReleaseCandidate;
        wallet.next_valid_actions =
            workloop::valid_actions_for_state(&workloop::LoopState::ReleaseCandidate);
        wallet.updated_at = workloop::Timestamp::now();
        wallet.revision += 1;

        if let Err(e) = workloop::save_wallet(workspace_root, &wallet) {
            return Err(mk_err(
                ErrorKind::Internal,
                &format!("Failed to save wallet: {e}"),
                &audit_id,
            ));
        }

        let result = ToolResult::ok(
            serde_json::to_value(&rc).unwrap_or_default(),
            format!("RC created: {} ({})", rc.repo_name, rc.change_id),
            &audit_id,
        );
        Ok(serde_json::to_value(&result).unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------
// Loop finalize-candidate tool
// ---------------------------------------------------------------------------

pub struct StitchLoopFinalizeCandidateTool;

impl McpTool for StitchLoopFinalizeCandidateTool {
    fn name(&self) -> &str {
        "stitch.loop_finalize_candidate"
    }
    fn description(&self) -> &str {
        "Finalize a release candidate (move main bookmark)"
    }
    fn metadata(&self) -> ToolMetadata {
        tool_meta(MutationLevel::CreatesCommit)
    }
    fn input_schema(&self) -> Value {
        json!({
            "repo": { "type": "string", "description": "Repo path (default: current dir)" },
            "feature": { "type": "string", "description": "Feature name for the wallet" },
            "apply": { "type": "boolean", "description": "Must be true to execute" }
        })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();

        let apply = input
            .get("apply")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !apply {
            return Err(mk_err(
                ErrorKind::PolicyDenied,
                "Must set apply=true to finalize a release candidate",
                &audit_id,
            ));
        }

        let repo_path = input
            .get("repo")
            .and_then(|v| v.as_str())
            .map(|s| std::path::Path::new(s).to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let feature = input
            .get("feature")
            .and_then(|v| v.as_str())
            .unwrap_or("default");
        let repo_name = repo_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("default")
            .to_string();

        let cfg = match stitch::config::find_and_load() {
            Ok(c) => c,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::NotFound,
                    &format!("Config: {}", e),
                    &audit_id,
                ))
            }
        };
        let workspace_root = std::path::Path::new(&cfg.workspace);

        // Load wallet (must exist)
        let mut wallet = match workloop::load_wallet(workspace_root, feature) {
            Ok(w) => w,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::NotFound,
                    &format!("Wallet not found: {e}"),
                    &audit_id,
                ))
            }
        };

        // Get RC change ID from wallet
        let rc_change_id = wallet
            .repos
            .iter()
            .find(|r| r.name == repo_name)
            .and_then(|r| r.release_candidate_change_id.clone())
            .ok_or_else(|| {
                mk_err(
                    ErrorKind::NotFound,
                    "No release candidate found in wallet for this repo",
                    &audit_id,
                )
            })?;

        let backend = match workloop::detect_backend(&repo_path) {
            Ok(b) => b,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Backend detection failed: {e}"),
                    &audit_id,
                ))
            }
        };

        let rc_candidate = workloop::ReleaseCandidate {
            repo_name: repo_name.clone(),
            change_id: rc_change_id,
            commit_id: String::new(),
            exportable_git_commit_id: None,
            checks_status: workloop::CheckStatus::NotRun,
        };

        let finalized = match backend.finalize_candidate(&repo_path, rc_candidate) {
            Ok(f) => f,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Finalization failed: {e}"),
                    &audit_id,
                ))
            }
        };

        // Update wallet
        wallet.state = workloop::LoopState::ReleaseFixedPoint;
        wallet.next_valid_actions =
            workloop::valid_actions_for_state(&workloop::LoopState::ReleaseFixedPoint);
        wallet.updated_at = workloop::Timestamp::now();
        wallet.revision += 1;

        if let Err(e) = workloop::save_wallet(workspace_root, &wallet) {
            return Err(mk_err(
                ErrorKind::Internal,
                &format!("Failed to save wallet: {e}"),
                &audit_id,
            ));
        }

        let result = ToolResult::ok(
            serde_json::to_value(&finalized).unwrap_or_default(),
            format!(
                "RC finalized: {} ({})",
                finalized.repo_name, finalized.commit_id
            ),
            &audit_id,
        );
        Ok(serde_json::to_value(&result).unwrap_or_default())
    }
}

// ---------------------------------------------------------------------------
// Loop publish tool
// ---------------------------------------------------------------------------

pub struct StitchLoopPublishTool;

impl McpTool for StitchLoopPublishTool {
    fn name(&self) -> &str {
        "stitch.loop_publish"
    }
    fn description(&self) -> &str {
        "Publish feature changes (push to remotes)"
    }
    fn metadata(&self) -> ToolMetadata {
        tool_meta(MutationLevel::Network)
    }
    fn input_schema(&self) -> Value {
        json!({
            "repo": { "type": "string", "description": "Repo path (default: current dir)" },
            "feature": { "type": "string", "description": "Feature name for the wallet" }
        })
    }
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure> {
        let audit_id = ctx.audit.generate_id();
        let _repo_path = input
            .get("repo")
            .and_then(|v| v.as_str())
            .map(|s| std::path::Path::new(s).to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let feature = input
            .get("feature")
            .and_then(|v| v.as_str())
            .unwrap_or("default");

        let cfg = match stitch::config::find_and_load() {
            Ok(c) => c,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::NotFound,
                    &format!("Config: {}", e),
                    &audit_id,
                ))
            }
        };
        let workspace_root = std::path::Path::new(&cfg.workspace);

        // Load wallet (must exist)
        let mut wallet = match workloop::load_wallet(workspace_root, feature) {
            Ok(w) => w,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::NotFound,
                    &format!("Wallet not found: {e}"),
                    &audit_id,
                ))
            }
        };

        // Build publish refs from wallet repos
        let refs = workloop::PublishRefs {
            targets: wallet
                .repos
                .iter()
                .map(|r| workloop::PublishTarget {
                    name: r.name.clone(),
                    path: r.path.clone(),
                    bookmark: r.main_bookmark.clone(),
                })
                .collect(),
            repos: Vec::new(),
            main_bookmarks: Vec::new(),
        };

        // Use JjBackend (its publish method already handles Git-only fallback)
        let backend = workloop::JjBackend::new();
        let publish_result = match backend.publish(refs) {
            Ok(r) => r,
            Err(e) => {
                return Err(mk_err(
                    ErrorKind::Internal,
                    &format!("Publish failed: {e}"),
                    &audit_id,
                ))
            }
        };

        // Update wallet
        wallet.state = workloop::LoopState::Published;
        wallet.next_valid_actions =
            workloop::valid_actions_for_state(&workloop::LoopState::Published);
        wallet.updated_at = workloop::Timestamp::now();
        wallet.revision += 1;

        if let Err(e) = workloop::save_wallet(workspace_root, &wallet) {
            return Err(mk_err(
                ErrorKind::Internal,
                &format!("Failed to save wallet: {e}"),
                &audit_id,
            ));
        }

        let results: Vec<Value> = publish_result
            .pushed
            .iter()
            .map(|r| json!({"target": r, "success": true, "message": "pushed"}))
            .chain(
                publish_result
                    .failed
                    .iter()
                    .map(|(r, e)| json!({"target": r, "success": false, "message": e})),
            )
            .collect();

        let out = json!({
            "results": results,
            "pushed": publish_result.pushed,
            "failed_count": publish_result.failed.len()
        });

        let result = ToolResult::ok(
            out,
            format!(
                "Published: {} pushed, {} failed",
                publish_result.pushed.len(),
                publish_result.failed.len()
            ),
            &audit_id,
        );
        Ok(serde_json::to_value(&result).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn read_only_exec_options_capture_step_output() {
        let cfg = stitch::model::WorkspaceConfig {
            version: 1,
            workspace: "test".to_string(),
            repos: Vec::new(),
            config_dir: None,
        };
        let plan = exec::ExecutionPlan {
            nodes: vec![exec::ExecutionNode {
                name: "repo".to_string(),
                path: PathBuf::from("/tmp"),
                role: None,
                layer: 0,
                directly_selected: true,
                directly_changed: false,
                downstream_only: false,
                steps: vec![exec::ExecutionStep {
                    id: "emit".to_string(),
                    mode: exec::ExecutionMode::ReadOnly,
                    kind: exec::StepKind::Shell {
                        argv: vec![
                            "sh".to_string(),
                            "-c".to_string(),
                            "printf mcp-visible-output".to_string(),
                        ],
                    },
                    condition: None,
                }],
            }],
        };

        let report = exec::run_plan(&cfg, &plan, &capture_exec_run_options()).unwrap();

        let step = &report.node_results[0].step_results[0];
        assert!(step.success);
        assert_eq!(step.stdout, "mcp-visible-output");
    }
}
