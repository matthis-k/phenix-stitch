mod tools;

use phenix_mcp_core::audit::AuditSink;
use phenix_mcp_core::mcp::{McpServer, ToolContext};
use phenix_mcp_core::roots::{McpRoot, RootValidator};
use phenix_mcp_core::runner::CommandRunner;
use phenix_mcp_core::safety::SafetyPolicy;
use std::path::PathBuf;

fn main() {
    let audit_dir = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
        .join(".local/share/phenix/audit/stitch-mcp");
    let cwd = std::env::current_dir().unwrap_or_default();
    let context = ToolContext {
        roots: RootValidator::new(vec![McpRoot::new(cwd, false)]),
        runner: CommandRunner::new(),
        audit: AuditSink::new(Some(audit_dir)),
        safety: SafetyPolicy::default(),
        server_name: "stitch-mcp".to_string(),
        server_version: "0.1.0".to_string(),
    };
    let mut server = McpServer::new(context);
    server.add_tool(Box::new(tools::StitchWorkspaceDiscoverTool));
    server.add_tool(Box::new(tools::StitchWorkspaceInventoryTool));
    server.add_tool(Box::new(tools::StitchGraphDeriveTool));
    server.add_tool(Box::new(tools::StitchGraphVerifyTool));
    server.add_tool(Box::new(tools::StitchGraphOrderTool));
    server.add_tool(Box::new(tools::StitchStatusTool));
    server.add_tool(Box::new(tools::StitchExecTool));
    server.run();
}
