use std::collections::{BTreeMap, HashMap};

use phenix_mcp_core::mcp::{McpTool, ToolContext};
use phenix_mcp_core::result::{ErrorKind, ToolFailure, ToolResult};
use phenix_mcp_core::types::{MutationLevel, ToolMetadata};
use serde_json::{json, Value};

use stitch::exec;

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
    let opts = exec::RunOptions {
        dry_run: false,
        apply: false,
        json: false,
    };
    exec::run_plan(cfg, &plan, &opts)
}

fn collect_status_json(cfg: &stitch::model::WorkspaceConfig, repo_filter: &[String]) -> Vec<Value> {
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
    let report = match run_exec_plan(
        cfg,
        selection,
        repo_filter.to_vec(),
        exec::ClosureMode::SelfOnly,
        exec::OrderMode::Stable,
        step,
    ) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
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
    statuses
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

        let statuses = collect_status_json(&cfg, &repo_filter);
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

        let statuses = collect_status_json(&cfg, &repo_filter);
        let mut nodes: Vec<Value> = Vec::new();

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

            if run_tend && mode != "sync" {
                nodes.push(json!({
                    "id": format!("{}:precheck", name),
                    "kind": "check",
                    "repo": name,
                    "command": ["tend", "run", "--changed"],
                    "depends_on": []
                }));
            }

            match split {
                "by-path" => {
                    let mut by_dir: HashMap<String, Vec<String>> = HashMap::new();
                    for f in &diff {
                        let dir = f
                            .rfind('/')
                            .map(|i| f[..i].to_string())
                            .unwrap_or_else(|| "root".to_string());
                        by_dir.entry(dir).or_default().push(f.clone());
                    }
                    for (dir, files) in &by_dir {
                        let deps = if run_tend && mode != "sync" {
                            vec![format!("{}:precheck", name)]
                        } else {
                            vec![]
                        };
                        nodes.push(json!({
                            "id": format!("{}:{}", name, dir.replace('/', "_")),
                            "kind": "commit",
                            "repo": name,
                            "files": files,
                            "depends_on": deps
                        }));
                    }
                }
                _ => {
                    let deps = if run_tend && mode != "sync" {
                        vec![format!("{}:precheck", name)]
                    } else {
                        vec![]
                    };
                    nodes.push(json!({
                        "id": format!("{}:commit", name),
                        "kind": "commit",
                        "repo": name,
                        "files": diff,
                        "depends_on": deps
                    }));
                }
            }
        }

        if mode == "full" || mode == "sync" {
            let commit_ids: Vec<String> = nodes
                .iter()
                .filter(|n| n["kind"] == "commit")
                .filter_map(|n| n["id"].as_str().map(|s| s.to_string()))
                .collect();

            if !commit_ids.is_empty() {
                let root_repo = cfg
                    .repos
                    .iter()
                    .find(|r| r.name.contains("root") || r.name == "phenix");
                if let Some(root) = root_repo {
                    nodes.push(json!({
                        "id": format!("{}:update-pins", root.name),
                        "kind": "update-pins",
                        "repo": root.name,
                        "files": ["flake.lock"],
                        "depends_on": commit_ids
                    }));
                }
            }
        }

        let result = ToolResult::ok(
            json!({ "nodes": nodes, "total": nodes.len(), "mode": mode }),
            format!("DAG: {} node(s) in {} mode", nodes.len(), mode),
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

        let cfg = stitch::config::find_and_load().unwrap_or(stitch::model::WorkspaceConfig {
            version: 1,
            workspace: ".".to_string(),
            repos: vec![],
            config_dir: None,
        });

        let statuses = collect_status_json(&cfg, &[]);
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

        let scope = exec::ExecutionScope {
            selection: exec::SelectionMode::Changed,
            explicit_nodes: Vec::new(),
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

        let dirty_nodes: Vec<&exec::ExecutionNode> =
            raw_nodes.iter().filter(|n| n.directly_changed).collect();

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
