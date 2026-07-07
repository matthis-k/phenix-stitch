mod prompts;
mod resources;
mod tools;

use std::path::PathBuf;

use phenix_mcp_core::audit::AuditSink;
use phenix_mcp_core::mcp::{McpServer, ToolContext};
use phenix_mcp_core::roots::{McpRoot, RootValidator};
use phenix_mcp_core::runner::CommandRunner;
use phenix_mcp_core::safety::SafetyPolicy;

fn main() {
    let audit_dir = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
        .join(".local")
        .join("share")
        .join("phenix")
        .join("audit")
        .join("stitch-mcp");

    let cwd = std::env::current_dir().unwrap_or_default();
    let roots = vec![McpRoot::new(cwd.clone(), false)];
    let validator = RootValidator::new(roots);

    let context = ToolContext {
        roots: validator,
        runner: CommandRunner::new(),
        audit: AuditSink::new(Some(audit_dir)),
        safety: SafetyPolicy {
            allow_commit: true,
            allow_network: true,
            ..SafetyPolicy::default()
        },
        server_name: "stitch-mcp".to_string(),
        server_version: "0.1.0".to_string(),
    };

    let mut server = McpServer::new(context);

    server.add_tool(Box::new(tools::StitchStatusTool));
    server.add_tool(Box::new(tools::StitchDiffTool));
    server.add_tool(Box::new(tools::StitchDagTool));
    server.add_tool(Box::new(tools::StitchWorkspaceDiscoverTool));
    server.add_tool(Box::new(tools::StitchWorkspaceDagPlanTool));
    server.add_tool(Box::new(tools::StitchWorkspaceVerifyPlanTool));
    server.add_tool(Box::new(tools::StitchWorkspaceLockStatusTool));
    server.add_tool(Box::new(tools::StitchCommitTemplateTool));
    server.add_tool(Box::new(tools::StitchCommitTool));
    server.add_tool(Box::new(tools::StitchSyncTool));
    server.add_tool(Box::new(tools::StitchLoopDetectTool));
    server.add_tool(Box::new(tools::StitchLoopSnapshotTool));
    server.add_tool(Box::new(tools::StitchLoopCheckpointTool));
    server.add_tool(Box::new(tools::StitchLoopCurrentChangeTool));
    server.add_tool(Box::new(tools::StitchLoopCreateRcTool));
    server.add_tool(Box::new(tools::StitchLoopFinalizeCandidateTool));
    server.add_tool(Box::new(tools::StitchLoopPublishTool));

    server.add_resource(Box::new(resources::StitchWorkspaceResource));
    server.add_resource(Box::new(resources::StitchReposResource));
    server.add_resource(Box::new(resources::StitchCommitSessionsResource));

    server.add_prompt(Box::new(prompts::StitchPlanSyncPrompt));
    server.add_prompt(Box::new(prompts::StitchSplitFeatureCommitsPrompt));

    server.run();
}
