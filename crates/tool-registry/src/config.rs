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

/// Serializable configuration for the memory backend (used in supervisor TOML).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryBackendConfig {
    /// Root directory for the memory store.
    /// Defaults to `$XDG_DATA_HOME/pince/memory/`.
    #[serde(default)]
    pub store_path: Option<PathBuf>,
    /// Command to spawn (default: `"qmd"`).
    #[serde(default = "default_qmd_command")]
    pub command: String,
    /// Arguments to pass to the command (default: `["mcp"]`).
    #[serde(default = "default_qmd_args")]
    pub args: Vec<String>,
    /// Optional environment variables for the backend process.
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

fn default_qmd_command() -> String {
    "qmd".into()
}

fn default_qmd_args() -> Vec<String> {
    vec!["mcp".into()]
}

impl Default for MemoryBackendConfig {
    fn default() -> Self {
        Self {
            store_path: None,
            command: default_qmd_command(),
            args: default_qmd_args(),
            env: Default::default(),
        }
    }
}

impl MemoryBackendConfig {
    /// Convert to a `MemoryConfig`, resolving the default store path if not set.
    pub fn into_memory_config(self) -> crate::memory::MemoryConfig {
        let store_path = self.store_path.unwrap_or_else(|| {
            crate::memory::MemoryConfig::default_config().store_path
        });
        crate::memory::MemoryConfig {
            store_path,
            command: self.command,
            args: self.args,
            env: self.env,
        }
    }
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

    /// Memory backend configuration.
    #[serde(default)]
    pub memory: MemoryBackendConfig,
}

impl ToolRegistryConfig {
    /// Parse config from a TOML string.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }
}
