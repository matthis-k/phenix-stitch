use phenix_mcp_core::mcp::{McpResource, ToolContext};
use phenix_mcp_core::result::ToolFailure;
use serde_json::{json, Value};

pub struct StitchWorkspaceResource;

impl McpResource for StitchWorkspaceResource {
    fn uri(&self) -> &str {
        "stitch://workspace"
    }
    fn name(&self) -> &str {
        "Stitch Workspace"
    }
    fn description(&self) -> &str {
        "Current workspace configuration and repo list"
    }
    fn mime_type(&self) -> &str {
        "application/json"
    }
    fn read(&self, _ctx: &ToolContext) -> Result<Value, ToolFailure> {
        match stitch::config::find_and_load() {
            Ok(cfg) => {
                let repos: Vec<Value> = cfg
                    .repos
                    .iter()
                    .map(|r| json!({"name": r.name, "path": r.path}))
                    .collect();
                Ok(json!({
                    "workspace": cfg.workspace,
                    "repos": repos
                }))
            }
            Err(e) => Ok(json!({"error": e.to_string()})),
        }
    }
}

pub struct StitchReposResource;

impl McpResource for StitchReposResource {
    fn uri(&self) -> &str {
        "stitch://repos"
    }
    fn name(&self) -> &str {
        "Stitch Repos"
    }
    fn description(&self) -> &str {
        "Status overview of all repos in the workspace"
    }
    fn mime_type(&self) -> &str {
        "application/json"
    }
    fn read(&self, _ctx: &ToolContext) -> Result<Value, ToolFailure> {
        match stitch::config::find_and_load() {
            Ok(cfg) => {
                let statuses = stitch::status::collect_all(&cfg).unwrap_or_default();
                let repos: Vec<Value> = statuses
                    .iter()
                    .map(|s| {
                        json!({
                            "name": s.name,
                            "branch": s.branch,
                            "dirty": s.is_dirty,
                            "staged": s.staged_count,
                            "unstaged": s.unstaged_count,
                            "untracked": s.untracked_count
                        })
                    })
                    .collect();
                Ok(json!({ "repos": repos }))
            }
            Err(e) => Ok(json!({"error": e.to_string(), "repos": []})),
        }
    }
}

pub struct StitchCommitSessionsResource;

impl McpResource for StitchCommitSessionsResource {
    fn uri(&self) -> &str {
        "stitch://commit-sessions"
    }
    fn name(&self) -> &str {
        "Commit Sessions"
    }
    fn description(&self) -> &str {
        "Active and recent commit sessions"
    }
    fn mime_type(&self) -> &str {
        "application/json"
    }
    fn read(&self, _ctx: &ToolContext) -> Result<Value, ToolFailure> {
        Ok(json!({"sessions": [], "total": 0}))
    }
}
