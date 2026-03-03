//! Tool handlers for the four memory tools.
//!
//! Each handler implements `ToolHandler` and delegates to the `MemoryBackend`
//! shared via `Arc<dyn MemoryBackend>`.
//!
//! **Path validation**: all handlers normalize paths and reject anything that
//! escapes the memory store directory (path traversal prevention).

use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::{
    ToolError, ToolHandler, ToolOutput,
    schema::{RiskLevel, ToolSchema},
};

use super::MemoryBackend;

// ── Path helpers ─────────────────────────────────────────────────────────────

/// Normalize a relative path, rejecting `..` components and absolute paths.
///
/// Returns `ToolError::InvalidArguments` if the path is invalid.
fn normalize_path(raw: &str) -> Result<String, ToolError> {
    let path = Path::new(raw);

    if path.is_absolute() {
        return Err(ToolError::InvalidArguments(format!(
            "memory paths must be relative, got: '{raw}'"
        )));
    }

    let mut components: Vec<&str> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(os) => {
                let s = os.to_str().ok_or_else(|| {
                    ToolError::InvalidArguments("path component is not valid UTF-8".into())
                })?;
                components.push(s);
            }
            Component::ParentDir => {
                return Err(ToolError::InvalidArguments(format!(
                    "path traversal not allowed: '{raw}'"
                )));
            }
            Component::CurDir => {} // skip `.`
            Component::RootDir | Component::Prefix(_) => {
                return Err(ToolError::InvalidArguments(format!(
                    "absolute paths not allowed: '{raw}'"
                )));
            }
        }
    }

    if components.is_empty() {
        return Err(ToolError::InvalidArguments(
            "path must not be empty".into(),
        ));
    }

    Ok(PathBuf::from_iter(&components)
        .to_string_lossy()
        .into_owned())
}

// ── memory_search ─────────────────────────────────────────────────────────────

pub fn memory_search_schema() -> ToolSchema {
    ToolSchema::new(
        "memory_search",
        "Search persistent agent memory using hybrid keyword + semantic search. \
         Returns matching document snippets. Use this first when looking for past context.",
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query."
                },
                "collection": {
                    "type": "string",
                    "description": "Optional subdirectory to search within (e.g. 'knowledge', 'tasks')."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 10).",
                    "minimum": 1,
                    "maximum": 100
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
        RiskLevel::Safe,
    )
}

pub struct MemorySearchHandler {
    pub backend: Arc<dyn MemoryBackend>,
}

impl ToolHandler for MemorySearchHandler {
    fn execute<'a>(
        &'a self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + 'a>> {
        Box::pin(async move {
            let query = args["query"]
                .as_str()
                .ok_or_else(|| ToolError::InvalidArguments("missing 'query'".into()))?;

            let collection = match args["collection"].as_str() {
                Some(c) => Some(normalize_path(c)?),
                None => None,
            };
            let limit = args["limit"].as_u64().map(|v| v as u32);

            let results = self
                .backend
                .search(query, collection.as_deref(), limit)
                .await
                .map_err(ToolError::ExecutionFailed)?;

            let items: Vec<Value> = results
                .iter()
                .map(|r| {
                    json!({
                        "path": r.path,
                        "snippet": r.snippet,
                        "score": r.score
                    })
                })
                .collect();

            Ok(json!({ "results": items }))
        })
    }
}

// ── memory_get ────────────────────────────────────────────────────────────────

pub fn memory_get_schema() -> ToolSchema {
    ToolSchema::new(
        "memory_get",
        "Retrieve the full content of a memory document by its relative path.",
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path to the memory document (e.g. 'knowledge/preferences.md')."
                }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
        RiskLevel::Safe,
    )
}

pub struct MemoryGetHandler {
    pub backend: Arc<dyn MemoryBackend>,
}

impl ToolHandler for MemoryGetHandler {
    fn execute<'a>(
        &'a self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + 'a>> {
        Box::pin(async move {
            let raw_path = args["path"]
                .as_str()
                .ok_or_else(|| ToolError::InvalidArguments("missing 'path'".into()))?;

            let path = normalize_path(raw_path)?;

            let doc = self
                .backend
                .get(&path)
                .await
                .map_err(ToolError::ExecutionFailed)?;

            Ok(json!({
                "path": doc.path,
                "content": doc.content
            }))
        })
    }
}

// ── memory_store ──────────────────────────────────────────────────────────────

pub fn memory_store_schema() -> ToolSchema {
    ToolSchema::new(
        "memory_store",
        "Write or update a markdown document in persistent agent memory. \
         Creates any missing parent directories automatically.",
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path for the document (e.g. 'knowledge/user-preferences.md')."
                },
                "content": {
                    "type": "string",
                    "description": "Markdown content to write."
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        }),
        RiskLevel::Sensitive,
    )
}

pub struct MemoryStoreHandler {
    pub backend: Arc<dyn MemoryBackend>,
}

impl ToolHandler for MemoryStoreHandler {
    fn execute<'a>(
        &'a self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + 'a>> {
        Box::pin(async move {
            let raw_path = args["path"]
                .as_str()
                .ok_or_else(|| ToolError::InvalidArguments("missing 'path'".into()))?;

            let content = args["content"]
                .as_str()
                .ok_or_else(|| ToolError::InvalidArguments("missing 'content'".into()))?;

            let path = normalize_path(raw_path)?;

            let result = self
                .backend
                .store(&path, content)
                .await
                .map_err(ToolError::ExecutionFailed)?;

            let status = match result.status {
                super::types::StoreStatus::Created => "created",
                super::types::StoreStatus::Updated => "updated",
            };

            Ok(json!({
                "path": result.path,
                "status": status
            }))
        })
    }
}

// ── memory_list ───────────────────────────────────────────────────────────────

pub fn memory_list_schema() -> ToolSchema {
    ToolSchema::new(
        "memory_list",
        "List documents in persistent agent memory, optionally filtered by a glob pattern.",
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Optional glob pattern to filter results (e.g. 'knowledge/**/*.md')."
                }
            },
            "additionalProperties": false
        }),
        RiskLevel::Safe,
    )
}

pub struct MemoryListHandler {
    pub backend: Arc<dyn MemoryBackend>,
}

impl ToolHandler for MemoryListHandler {
    fn execute<'a>(
        &'a self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + 'a>> {
        Box::pin(async move {
            let pattern = match args["pattern"].as_str() {
                Some(p) => {
                    // Reject path traversal in glob patterns.
                    if p.contains("..") {
                        return Err(ToolError::InvalidArguments(format!(
                            "path traversal not allowed in pattern: '{p}'"
                        )));
                    }
                    Some(p)
                }
                None => None,
            };

            let docs = self
                .backend
                .list(pattern)
                .await
                .map_err(ToolError::ExecutionFailed)?;

            let items: Vec<Value> = docs
                .iter()
                .map(|d| {
                    json!({
                        "path": d.path,
                        "modified_at": d.modified_at,
                        "size": d.size
                    })
                })
                .collect();

            Ok(json!({ "documents": items }))
        })
    }
}

// ── Register all ─────────────────────────────────────────────────────────────

/// Register all four memory tools in the given `ToolRegistry`.
///
/// The `backend` is shared across all four handlers via `Arc`.
pub fn register_all(
    registry: &mut crate::ToolRegistry,
    backend: Arc<dyn MemoryBackend>,
) {
    registry.register(
        memory_search_schema(),
        Box::new(MemorySearchHandler { backend: Arc::clone(&backend) }),
    );
    registry.register(
        memory_get_schema(),
        Box::new(MemoryGetHandler { backend: Arc::clone(&backend) }),
    );
    registry.register(
        memory_store_schema(),
        Box::new(MemoryStoreHandler { backend: Arc::clone(&backend) }),
    );
    registry.register(
        memory_list_schema(),
        Box::new(MemoryListHandler { backend }),
    );
}
