use std::collections::HashMap;

use phenix_mcp_core::mcp::{McpPrompt, PromptArgument, ToolContext};
use phenix_mcp_core::result::ToolFailure;
use serde_json::{json, Value};

pub struct StitchPlanSyncPrompt;

impl McpPrompt for StitchPlanSyncPrompt {
    fn name(&self) -> &str {
        "stitch/plan-sync"
    }
    fn description(&self) -> &str {
        "Guide to planning a multi-repo sync"
    }
    fn arguments(&self) -> Vec<PromptArgument> {
        vec![]
    }
    fn get(&self, _args: HashMap<String, Value>, _ctx: &ToolContext) -> Result<Value, ToolFailure> {
        Ok(
            json!({"messages": [{"role": "user", "content": "I need to plan a sync across repos."}]}),
        )
    }
}

pub struct StitchSplitFeatureCommitsPrompt;

impl McpPrompt for StitchSplitFeatureCommitsPrompt {
    fn name(&self) -> &str {
        "stitch/split-feature-commits"
    }
    fn description(&self) -> &str {
        "Guide to splitting changes into coherent feature commits"
    }
    fn arguments(&self) -> Vec<PromptArgument> {
        vec![]
    }
    fn get(&self, _args: HashMap<String, Value>, _ctx: &ToolContext) -> Result<Value, ToolFailure> {
        Ok(
            json!({"messages": [{"role": "user", "content": "Help me split changes into coherent feature commits."}]}),
        )
    }
}
