#![allow(clippy::result_large_err)]

use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::audit::AuditSink;
use crate::result::ToolFailure;
use crate::roots::RootValidator;
use crate::runner::CommandRunner;
use crate::safety::SafetyPolicy;
use crate::types::ToolMetadata;

pub struct ToolContext {
    pub roots: RootValidator,
    pub runner: CommandRunner,
    pub audit: AuditSink,
    pub safety: SafetyPolicy,
    pub server_name: String,
    pub server_version: String,
}

pub trait McpTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn metadata(&self) -> ToolMetadata;
    fn input_schema(&self) -> Value;
    fn call(&self, input: Value, ctx: &ToolContext) -> Result<Value, ToolFailure>;
}

pub trait McpResource: Send + Sync {
    fn uri(&self) -> &str;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn mime_type(&self) -> &str;
    fn read(&self, ctx: &ToolContext) -> Result<Value, ToolFailure>;
}

pub trait McpPrompt: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn arguments(&self) -> Vec<PromptArgument>;
    fn get(&self, args: HashMap<String, Value>, ctx: &ToolContext) -> Result<Value, ToolFailure>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptArgument {
    pub name: String,
    pub description: String,
    pub required: bool,
}

fn extract_rootable_paths(input: &Value) -> Vec<std::path::PathBuf> {
    const PATH_KEYS: &[&str] = &[
        "root",
        "path",
        "repo",
        "cwd",
        "dir",
        "directory",
        "workspace_root",
    ];

    let mut paths = Vec::new();
    for key in PATH_KEYS {
        if let Some(value) = input.get(*key).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                paths.push(std::path::PathBuf::from(value));
            }
        }
    }
    paths
}

fn tool_to_json(tool: &dyn McpTool) -> Value {
    let meta = tool.metadata();
    let schema = tool.input_schema();

    // Determine required fields based on metadata
    let mut required: Vec<String> = Vec::new();

    // If tool requires apply=true and mutates, "apply" is required
    if meta.mutation.requires_apply() {
        required.push("apply".to_string());
    }

    // Check if schema itself has a "required" field from the tool
    if let Some(schema_req) = schema.get("required").and_then(|v| v.as_array()) {
        for r in schema_req {
            if let Some(s) = r.as_str() {
                if !required.contains(&s.to_string()) {
                    required.push(s.to_string());
                }
            }
        }
    }

    // Extract properties (without "required" key if present in schema)
    let mut properties_map = serde_json::Map::new();
    if let Some(obj) = schema.as_object() {
        for (k, v) in obj {
            if k != "required" {
                properties_map.insert(k.clone(), v.clone());
            }
        }
    }

    // Inject "apply" property schema when required but not declared by tool
    if meta.mutation.requires_apply() && !properties_map.contains_key("apply") {
        properties_map.insert(
            "apply".to_string(),
            json!({ "type": "boolean", "description": "Must be true to execute" }),
        );
    }

    let properties = Value::Object(properties_map);

    json!({
        "name": tool.name(),
        "description": tool.description(),
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required
        }
    })
}

fn resource_to_json(resource: &dyn McpResource) -> Value {
    json!({
        "uri": resource.uri(),
        "name": resource.name(),
        "description": resource.description(),
        "mimeType": resource.mime_type()
    })
}

fn prompt_to_json(prompt: &dyn McpPrompt) -> Value {
    let args: Vec<Value> = prompt
        .arguments()
        .iter()
        .map(|a| {
            json!({
                "name": a.name,
                "description": a.description,
                "required": a.required
            })
        })
        .collect();

    json!({
        "name": prompt.name(),
        "description": prompt.description(),
        "arguments": args
    })
}

pub struct McpServer {
    pub tools: Vec<Box<dyn McpTool>>,
    pub resources: Vec<Box<dyn McpResource>>,
    pub prompts: Vec<Box<dyn McpPrompt>>,
    pub context: ToolContext,
}

impl McpServer {
    pub fn new(context: ToolContext) -> Self {
        Self {
            tools: Vec::new(),
            resources: Vec::new(),
            prompts: Vec::new(),
            context,
        }
    }

    pub fn add_tool(&mut self, tool: Box<dyn McpTool>) {
        self.tools.push(tool);
    }

    pub fn add_resource(&mut self, resource: Box<dyn McpResource>) {
        self.resources.push(resource);
    }

    pub fn add_prompt(&mut self, prompt: Box<dyn McpPrompt>) {
        self.prompts.push(prompt);
    }

    pub fn run(&self) {
        let stdin = io::stdin();
        let stdout = io::stdout();

        for line in stdin.lock().lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };

            if line.trim().is_empty() {
                continue;
            }

            let request: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => {
                    let error = json!({
                        "jsonrpc": "2.0",
                        "id": null,
                        "error": {
                            "code": -32700,
                            "message": format!("Parse error: {}", e)
                        }
                    });
                    let mut out = stdout.lock();
                    let _ = writeln!(out, "{}", serde_json::to_string(&error).unwrap());
                    let _ = out.flush();
                    continue;
                }
            };

            let response = self.handle_request(&request);
            if let Some(resp) = response {
                let mut out = stdout.lock();
                let _ = writeln!(out, "{}", serde_json::to_string(&resp).unwrap());
                let _ = out.flush();
            }
        }
    }

    fn handle_request(&self, request: &Value) -> Option<Value> {
        let method = request.get("method")?.as_str()?;
        let id = request.get("id");
        let empty_params = json!({});
        let params = request.get("params").unwrap_or(&empty_params);

        match method {
            "initialize" => {
                let protocol_version = params
                    .get("protocolVersion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("2025-11-05");

                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": protocol_version,
                        "capabilities": {
                            "tools": {},
                            "resources": {},
                            "prompts": {},
                            "roots": {
                                "listChanged": false
                            },
                            "sampling": {}
                        },
                        "serverInfo": {
                            "name": self.context.server_name,
            "notifications/cancelled" => None,
            "tools/list" => {
                let tools_json: Vec<Value> = self
                    .tools
                    .iter()
                    .map(|t| tool_to_json(t.as_ref()))
                    .collect();
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": tools_json }
                }))
            }
            "tools/call" => {
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let empty_args = json!({});
                let arguments = params.get("arguments").unwrap_or(&empty_args);

                let tool = self.tools.iter().find(|t| t.name() == name);

                match tool {
                    Some(t) => {
                        let _audit_id = self.context.audit.generate_id();
                        let meta = t.metadata();

                        if let Err(e) = self.context.safety.check_mutation(&meta.mutation) {
                            let failure = ToolFailure::new(
                                crate::result::ErrorKind::PolicyDenied,
                                &e,
                                &_audit_id,
                            );
                            return Some(json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "content": [{
                                        "type": "text",
                                        "text": serde_json::to_string(&failure).unwrap_or_default()
                                    }],
                                    "isError": true
                                }
                            }));
                        }

                        if meta.allowed_roots_only == Some(true)
                            && !self.context.roots.roots().is_empty()
                        {
                            let root_paths = extract_rootable_paths(arguments);
                            let active_root = &self.context.roots.roots()[0];
                            for p in &root_paths {
                                let resolved = if p.is_relative() {
                                    active_root.path.join(p)
                                } else {
                                    p.clone()
                                };
                                if let Err(e) = self.context.roots.validate_path(&resolved) {
                                    let failure = ToolFailure::new(
                                        crate::result::ErrorKind::RootViolation,
                                        format!(
                                            "Path '{}' is outside declared roots: {}",
                                            p.display(),
                                            e
                                        ),
                                        &_audit_id,
                                    );
                                    return Some(json!({
                                        "jsonrpc": "2.0",
                                        "id": id,
                                        "result": {
                                            "content": [{
                                                "type": "text",
                                                "text": serde_json::to_string(&failure).unwrap_or_default()
                                            }],
                                            "isError": true
                                        }
                                    }));
                                }
                            }
                        }

                        if meta.mutation.requires_apply() {
                            let apply = arguments
                                .get("apply")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if !apply {
                                let failure = ToolFailure::new(
                                    crate::result::ErrorKind::PolicyDenied,
                                    "Mutating tool requires apply=true",
                                    &_audit_id,
                                );
                                return Some(json!({
                                    "jsonrpc": "2.0",
                                    "id": id,
                                    "result": {
                                        "content": [{
                                            "type": "text",
                                            "text": serde_json::to_string(&failure).unwrap_or_default()
                                        }],
                                        "isError": true
                                    }
                                }));
                            }
                        }

                        let result = t.call(arguments.clone(), &self.context);

                        match result {
                            Ok(data) => Some(json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "content": [
                                        {
                                            "type": "text",
                                            "text": serde_json::to_string(&data).unwrap_or_default()
                                        }
                                    ],
                                    "isError": false
                                }
                            })),
                            Err(failure) => Some(json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": {
                                    "content": [
                                        {
                                            "type": "text",
                                            "text": serde_json::to_string(&failure).unwrap_or_default()
                                        }
                                    ],
                                    "isError": true
                                }
                            })),
                        }
                    }
                    None => Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32601,
                            "message": format!("Tool not found: {}", name)
                        }
                    })),
                }
            }
            "resources/list" => {
                let resources_json: Vec<Value> = self
                    .resources
                    .iter()
                    .map(|r| resource_to_json(r.as_ref()))
                    .collect();
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "resources": resources_json }
                }))
            }
            "resources/read" => {
                let uri = params.get("uri").and_then(|v| v.as_str()).unwrap_or("");

                let resource = self.resources.iter().find(|r| r.uri() == uri);

                match resource {
                    Some(r) => match r.read(&self.context) {
                        Ok(data) => Some(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "contents": [
                                    {
                                        "uri": r.uri(),
                                        "mimeType": r.mime_type(),
                                        "text": serde_json::to_string(&data).unwrap_or_default()
                                    }
                                ]
                            }
                        })),
                        Err(failure) => Some(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": {
                                "code": -32603,
                                "message": failure.summary
                            }
                        })),
                    },
                    None => Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32602,
                            "message": format!("Resource not found: {}", uri)
                        }
                    })),
                }
            }
            "prompts/list" => {
                let prompts_json: Vec<Value> = self
                    .prompts
                    .iter()
                    .map(|p| prompt_to_json(p.as_ref()))
                    .collect();
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "prompts": prompts_json }
                }))
            }
            "prompts/get" => {
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let args_map: HashMap<String, Value> =
                    serde_json::from_value(args).unwrap_or_default();

                let prompt = self.prompts.iter().find(|p| p.name() == name);

                match prompt {
                    Some(p) => match p.get(args_map, &self.context) {
                        Ok(data) => Some(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "messages": [
                                    {
                                        "role": "user",
                                        "content": {
                                            "type": "text",
                                            "text": serde_json::to_string(&data).unwrap_or_default()
                                        }
                                    }
                                ]
                            }
                        })),
                        Err(failure) => Some(json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": {
                                "code": -32603,
                                "message": failure.summary
                            }
                        })),
                    },
                    None => Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32602,
                            "message": format!("Prompt not found: {}", name)
                        }
                    })),
                }
            }
            _ => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Method not found: {}", method)
                }
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::extract_rootable_paths;
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn repo_argument_is_subject_to_root_validation() {
        let paths = extract_rootable_paths(&json!({
            "repo": "../outside",
            "unrelated": "ignored"
        }));
        assert_eq!(paths, vec![PathBuf::from("../outside")]);
    }

    #[test]
    fn all_supported_path_fields_are_collected() {
        let paths = extract_rootable_paths(&json!({
            "root": "root",
            "path": "path",
            "repo": "repo",
            "cwd": "cwd",
            "dir": "dir",
            "directory": "directory",
            "workspace_root": "workspace",
            "empty": ""
        }));
        assert_eq!(paths.len(), 7);
    }
}
