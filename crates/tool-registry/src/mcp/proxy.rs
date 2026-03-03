//! `McpToolHandler` — proxies a single MCP tool call through an `McpClient`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::Value;

use crate::{ToolError, ToolHandler, ToolOutput};

use super::client::McpClient;

/// A `ToolHandler` that proxies calls to an MCP server.
pub struct McpToolHandler {
    pub client: Arc<McpClient>,
    pub tool_name: String,
}

impl ToolHandler for McpToolHandler {
    fn execute<'a>(
        &'a self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + 'a>> {
        Box::pin(async move {
            self.client
                .call_tool(&self.tool_name, args)
                .await
                .map_err(ToolError::ExecutionFailed)
        })
    }
}
