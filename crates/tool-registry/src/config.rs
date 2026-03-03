//! Configuration for the tool registry.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Logical name for this MCP server (used for logging).
    pub name: String,
    /// Command to spawn (e.g. `"qmd"`).
    pub command: String,
    /// Arguments to pass to the command.
    #[serde(default)]
    pub args: Vec<String>,
    /// Optional environment variables for the MCP process.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

/// Top-level tool registry configuration (from supervisor TOML).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolRegistryConfig {
    /// MCP servers to spawn and load tools from.
    #[serde(default, rename = "mcp_servers")]
    pub mcp_servers: Vec<McpServerConfig>,

    /// Protected filesystem paths (secrets dir, etc.).
    /// Tools that access the filesystem will deny access to these paths.
    #[serde(default)]
    pub protected_paths: Vec<PathBuf>,
}

impl ToolRegistryConfig {
    /// Parse config from a TOML string.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }
}
