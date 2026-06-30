//! MCP (Model Context Protocol) Server for CKC.
//!
//! Implements JSON-RPC 2.0 over stdio, exposing CKC knowledge graph
//! queries as MCP tools consumable by Claude, Copilot, and other AI agents.

use ckc_graph::GraphStore;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

// ── JSON-RPC Types ─────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

#[derive(Serialize)]
#[derive(Serialize)]
struct Tool {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: serde_json::Value,
}

// ── MCP Server ─────────────────────────────────────────────────────────────

pub struct McpServer {
    store: GraphStore,
}

impl McpServer {
    pub fn new(db_path: &PathBuf) -> Result<Self, anyhow::Error> {
        let store = GraphStore::open(db_path)?;
        Ok(Self { store })
    }

    pub fn run(&self) -> Result<(), anyhow::Error> {
        let stdin = std::io::stdin();
        let reader = BufReader::new(stdin.lock());
        let mut stdout = std::io::stdout();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            let response = match serde_json::from_str::<JsonRpcRequest>(&line) {
                Ok(req) => self.handle_request(&req),
                Err(e) => JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id: None,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {}", e),
                    }),
                },
            };

            let resp_json = serde_json::to_string(&response)?;
            writeln!(stdout, "{}", resp_json)?;
            stdout.flush()?;
        }
        Ok(())
    }

    fn handle_request(&self, req: &JsonRpcRequest) -> JsonRpcResponse {
        match req.method.as_str() {
            "initialize" => self.handle_initialize(req),
            "notifications/initialized" => JsonRpcResponse {
                jsonrpc: "2.0".into(), id: None, result: Some(serde_json::json!({})), error: None,
            },
            "tools/list" => self.handle_tools_list(req),
            "tools/call" => self.handle_tools_call(req),
            _ => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id: req.id.clone(),
                result: None,
                error: Some(JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {}", req.method),
                }),
            },
        }
    }

    fn handle_initialize(&self, req: &JsonRpcRequest) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: req.id.clone(),
            result: Some(serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": "ckc-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
            error: None,
        }
    }

    fn handle_tools_list(&self, req: &JsonRpcRequest) -> JsonRpcResponse {
        let tools = vec![
            Tool {
                name: "ckc_search".into(),
                description: "Full-text search for symbols (functions, classes, methods) in the codebase.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query (FTS5 syntax, use * for prefix)" }
                    },
                    "required": ["query"]
                }),
            },
            Tool {
                name: "ckc_callers".into(),
                description: "Find all callers of a given function or method.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Symbol name" },
                        "depth": { "type": "integer", "description": "Traversal depth (default 1)", "default": 1 }
                    },
                    "required": ["name"]
                }),
            },
            Tool {
                name: "ckc_callees".into(),
                description: "Find all functions/methods called by a given symbol.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Symbol name" },
                        "depth": { "type": "integer", "description": "Traversal depth (default 1)", "default": 1 }
                    },
                    "required": ["name"]
                }),
            },
            Tool {
                name: "ckc_imports".into(),
                description: "List all imports for a given file.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "file": { "type": "string", "description": "File path relative to repo root" }
                    },
                    "required": ["file"]
                }),
            },
            Tool {
                name: "ckc_status".into(),
                description: "Get build statistics for the compiled knowledge graph.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {}
                }),
            },
        ];

        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: req.id.clone(),
            result: Some(serde_json::json!({ "tools": tools })),
            error: None,
        }
    }

    fn handle_tools_call(&self, req: &JsonRpcRequest) -> JsonRpcResponse {
        let params = match &req.params {
            Some(p) => p,
            None => return self.error_response(req, -32602, "Missing params"),
        };

        let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let args = params.get("arguments").cloned().unwrap_or(serde_json::json!({}));

        let result = match tool_name {
            "ckc_search" => {
                let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                match self.store.search(q) {
                    Ok(nodes) => serde_json::json!({
                        "content": [{ "type": "text", "text": format!("Found {} result(s)", nodes.len()) }],
                        "data": nodes
                    }),
                    Err(e) => serde_json::json!({ "error": e.to_string() }),
                }
            }
            "ckc_callers" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
                match self.store.callers(name, depth) {
                    Ok(nodes) => serde_json::json!({
                        "content": [{ "type": "text", "text": format!("{} caller(s) of '{}'", nodes.len(), name) }],
                        "data": nodes
                    }),
                    Err(e) => serde_json::json!({ "error": e.to_string() }),
                }
            }
            "ckc_callees" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
                match self.store.callees(name, depth) {
                    Ok(nodes) => serde_json::json!({
                        "content": [{ "type": "text", "text": format!("{} callee(s) of '{}'", nodes.len(), name) }],
                        "data": nodes
                    }),
                    Err(e) => serde_json::json!({ "error": e.to_string() }),
                }
            }
            "ckc_imports" => {
                let file = args.get("file").and_then(|v| v.as_str()).unwrap_or("");
                match self.store.imports_of_file(file) {
                    Ok(edges) => {
                        let names: Vec<String> = edges.iter().map(|e| e.target_id.name.clone()).collect();
                        serde_json::json!({
                            "content": [{ "type": "text", "text": format!("{} import(s): {}", names.len(), names.join(", ")) }],
                            "data": edges
                        })
                    }
                    Err(e) => serde_json::json!({ "error": e.to_string() }),
                }
            }
            "ckc_status" => {
                serde_json::json!({
                    "content": [{
                        "type": "text",
                        "text": format!("{} nodes, {} edges, IR v{}",
                            self.store.total_nodes(),
                            self.store.total_edges(),
                            self.store.get_meta("ir_version").unwrap_or_default())
                    }]
                })
            }
            _ => return self.error_response(req, -32601, &format!("Unknown tool: {}", tool_name)),
        };

        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: req.id.clone(),
            result: Some(result),
            error: None,
        }
    }

    fn error_response(&self, req: &JsonRpcRequest, code: i32, message: &str) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".into(),
            id: req.id.clone(),
            result: None,
            error: Some(JsonRpcError { code, message: message.into() }),
        }
    }
}
