//! MCP (Model Context Protocol) client for stdio transport.
//!
//! Implements the minimal subset needed to:
//! 1. Spawn an MCP server as a child process.
//! 2. Send `initialize` and `tools/list` to discover tools.
//! 3. Register each discovered tool in the `ToolRegistry`.
//! 4. Proxy `tools/call` requests to the MCP server.

pub mod client;
pub mod loader;
pub mod proxy;

pub use client::McpClient;
pub use loader::load_mcp_server;
pub use proxy::McpToolHandler;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── JSON-RPC 2.0 types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id: Value::Number(id.into()),
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// An MCP tool descriptor returned by `tools/list`.
#[derive(Debug, Clone, Deserialize)]
pub struct McpTool {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}
