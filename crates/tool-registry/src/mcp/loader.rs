//! MCP server loader — spawns servers, discovers tools, registers them.

use std::sync::Arc;

use anyhow::Result;

use crate::{
    ToolRegistry,
    config::McpServerConfig,
    schema::{RiskLevel, ToolSchema},
};

use super::{McpClient, McpToolHandler};

/// Spawn an MCP server, discover its tools, and register them in the registry.
///
/// Tools are registered with a `safe` risk level by default; the permission
/// engine can override this with policy rules.
pub async fn load_mcp_server(
    registry: &mut ToolRegistry,
    config: &McpServerConfig,
) -> Result<Arc<McpClient>> {
    tracing::info!(
        server = %config.name,
        command = %config.command,
        "loading MCP server"
    );

    let client = Arc::new(
        McpClient::spawn(&config.command, &config.args, &config.env)
            .await?,
    );

    let tools = client.list_tools().await?;
    tracing::info!(server = %config.name, count = tools.len(), "discovered MCP tools");

    for tool in tools {
        let description = tool.description.unwrap_or_else(|| format!("{} (MCP)", tool.name));
        let schema = ToolSchema::new(
            tool.name.clone(),
            description,
            tool.input_schema,
            RiskLevel::Safe,
        );
        let handler = McpToolHandler {
            client: client.clone(),
            tool_name: tool.name.clone(),
        };
        tracing::debug!(tool = %tool.name, server = %config.name, "registering MCP tool");
        registry.register(schema, Box::new(handler));
    }

    Ok(client)
}
