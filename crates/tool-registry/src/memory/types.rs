//! Shared types for the memory subsystem.

use std::collections::HashMap;
use std::path::PathBuf;

/// Configuration for the memory backend.
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    /// Root directory for the memory store (e.g. `~/.local/share/pince/memory/`).
    pub store_path: PathBuf,
    /// Command to spawn (e.g. `"qmd"`).
    pub command: String,
    /// Arguments to pass to the command (e.g. `["mcp"]`).
    pub args: Vec<String>,
    /// Optional environment variables for the backend process.
    pub env: HashMap<String, String>,
}

impl MemoryConfig {
    /// Construct a config with default values, reading XDG env vars.
    pub fn default_config() -> Self {
        let store_path = {
            let data_home = std::env::var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
                    PathBuf::from(home).join(".local").join("share")
                });
            data_home.join("pince").join("memory")
        };
        Self {
            store_path,
            command: "qmd".into(),
            args: vec!["mcp".into()],
            env: HashMap::new(),
        }
    }
}

/// A single search result returned by the memory backend.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Relative path to the document within the memory store.
    pub path: String,
    /// A short text excerpt matching the query.
    pub snippet: String,
    /// Relevance score (higher = more relevant).
    pub score: f64,
}

/// A full document retrieved from the memory store.
#[derive(Debug, Clone)]
pub struct Document {
    /// Relative path to the document within the memory store.
    pub path: String,
    /// Full markdown content of the document.
    pub content: String,
}

/// Result returned after storing a document.
#[derive(Debug, Clone)]
pub struct StoreResult {
    /// The path where the document was written.
    pub path: String,
    /// Whether the document was newly created or updated.
    pub status: StoreStatus,
}

/// Whether a store operation created a new document or updated an existing one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreStatus {
    Created,
    Updated,
}

/// Metadata about a document in the memory store.
#[derive(Debug, Clone)]
pub struct DocumentMeta {
    /// Relative path to the document within the memory store.
    pub path: String,
    /// Last-modified timestamp (ISO 8601 string or filesystem mtime).
    pub modified_at: String,
    /// File size in bytes.
    pub size: u64,
}

/// Health status of the memory backend process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendStatus {
    Running,
    Starting,
    Stopped,
    Error(String),
}
