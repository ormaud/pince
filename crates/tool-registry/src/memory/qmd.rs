//! `QmdBackend` — memory backend that wraps a `qmd mcp` child process.
//!
//! Spawns `qmd mcp --collection <store_path>` and speaks the MCP protocol
//! over stdio, reusing the existing `McpClient` transport.
//!
//! Uses `tokio::sync::Mutex` for interior mutability so the backend can be
//! shared as `Arc<dyn MemoryBackend>` across multiple tool handlers.

use std::pin::Pin;
use std::future::Future;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::mcp::client::McpClient;

use super::{
    MemoryBackend,
    types::{
        BackendStatus, Document, DocumentMeta, MemoryConfig, SearchResult, StoreResult,
        StoreStatus,
    },
};

/// Memory backend backed by a `qmd mcp` child process.
///
/// Call `QmdBackend::spawn(config).await` to start the backend.
pub struct QmdBackend {
    /// The MCP client; `None` before `spawn()` or after `shutdown()`.
    client: Arc<Mutex<Option<McpClient>>>,
}

impl QmdBackend {
    /// Spawn the qmd process and perform the MCP initialize handshake.
    ///
    /// Also creates the memory store directory structure.
    pub async fn spawn(config: &MemoryConfig) -> Result<Self> {
        // Create the memory store directory and standard subdirectories.
        for subdir in &["conversations", "knowledge", "tasks", "scratch"] {
            tokio::fs::create_dir_all(config.store_path.join(subdir))
                .await
                .with_context(|| {
                    format!(
                        "create memory subdir '{}'",
                        config.store_path.join(subdir).display()
                    )
                })?;
        }

        // Build the argument list: append the store path as the collection.
        let mut args = config.args.clone();
        args.push("--collection".into());
        args.push(
            config
                .store_path
                .to_str()
                .context("memory store path is not valid UTF-8")?
                .to_string(),
        );

        let client = McpClient::spawn(&config.command, &args, &config.env)
            .await
            .with_context(|| format!("spawn memory backend '{}'", config.command))?;

        Ok(Self {
            client: Arc::new(Mutex::new(Some(client))),
        })
    }

    /// Gracefully shut down the backend process.
    ///
    /// Drops the `McpClient`, which closes the child's stdin and causes it
    /// to exit. Subsequent calls to backend methods will return errors.
    pub async fn shutdown(&self) {
        let mut guard = self.client.lock().await;
        *guard = None; // drops McpClient → closes child stdin → child exits
    }

    /// Extract the first text item from a qmd MCP tool response.
    ///
    /// qmd returns `{"content": [{"type": "text", "text": "..."}]}`.
    fn extract_text(resp: &Value) -> Option<String> {
        resp["content"]
            .as_array()?
            .iter()
            .find(|item| item["type"] == "text")
            .and_then(|item| item["text"].as_str())
            .map(|s| s.to_string())
    }
}

impl MemoryBackend for QmdBackend {
    fn health_check<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<BackendStatus>> + Send + 'a>> {
        Box::pin(async move {
            let guard = self.client.lock().await;
            let client = match guard.as_ref() {
                None => return Ok(BackendStatus::Stopped),
                Some(c) => c,
            };

            // Use tools/list as a lightweight ping.
            match client.list_tools().await {
                Ok(_) => Ok(BackendStatus::Running),
                Err(e) => Ok(BackendStatus::Error(e.to_string())),
            }
        })
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        collection: Option<&'a str>,
        limit: Option<u32>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchResult>>> + Send + 'a>> {
        Box::pin(async move {
            let guard = self.client.lock().await;
            let client = guard
                .as_ref()
                .context("QmdBackend not running — call spawn() first")?;

            let mut args = json!({ "query": query });
            if let Some(col) = collection {
                args["collection"] = json!(col);
            }
            if let Some(lim) = limit {
                args["limit"] = json!(lim);
            }

            let resp = client
                .call_tool("search", args)
                .await
                .context("qmd search")?;

            // Parse the response. qmd may return a JSON array of results or
            // a text representation. Try structured response first.
            if let Some(results) = resp["results"].as_array() {
                let parsed = results
                    .iter()
                    .map(|r| SearchResult {
                        path: r["path"].as_str().unwrap_or("").to_string(),
                        snippet: r["snippet"].as_str().unwrap_or("").to_string(),
                        score: r["score"].as_f64().unwrap_or(0.0),
                    })
                    .collect();
                return Ok(parsed);
            }

            // Fall back: treat the text response as a single result.
            if let Some(text) = Self::extract_text(&resp) {
                if text.is_empty() {
                    return Ok(vec![]);
                }
                return Ok(vec![SearchResult {
                    path: String::new(),
                    snippet: text,
                    score: 1.0,
                }]);
            }

            Ok(vec![])
        })
    }

    fn get<'a>(
        &'a self,
        path: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Document>> + Send + 'a>> {
        Box::pin(async move {
            let guard = self.client.lock().await;
            let client = guard
                .as_ref()
                .context("QmdBackend not running — call spawn() first")?;

            let resp = client
                .call_tool("read", json!({ "path": path }))
                .await
                .context("qmd read")?;

            // Try structured response first.
            if let Some(content) = resp["content"].as_str() {
                return Ok(Document {
                    path: path.to_string(),
                    content: content.to_string(),
                });
            }

            // Try MCP text content array.
            if let Some(text) = Self::extract_text(&resp) {
                return Ok(Document {
                    path: path.to_string(),
                    content: text,
                });
            }

            bail!("qmd read: unexpected response format for '{path}'")
        })
    }

    fn store<'a>(
        &'a self,
        path: &'a str,
        content: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<StoreResult>> + Send + 'a>> {
        Box::pin(async move {
            let guard = self.client.lock().await;
            let client = guard
                .as_ref()
                .context("QmdBackend not running — call spawn() first")?;

            let resp = client
                .call_tool("write", json!({ "path": path, "content": content }))
                .await
                .context("qmd write")?;

            // Determine created vs updated from the response.
            let status = if resp["created"].as_bool().unwrap_or(false) {
                StoreStatus::Created
            } else if let Some(text) = Self::extract_text(&resp) {
                if text.to_lowercase().contains("creat") {
                    StoreStatus::Created
                } else {
                    StoreStatus::Updated
                }
            } else {
                StoreStatus::Updated
            };

            Ok(StoreResult {
                path: path.to_string(),
                status,
            })
        })
    }

    fn list<'a>(
        &'a self,
        pattern: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<DocumentMeta>>> + Send + 'a>> {
        Box::pin(async move {
            let guard = self.client.lock().await;
            let client = guard
                .as_ref()
                .context("QmdBackend not running — call spawn() first")?;

            let args = if let Some(p) = pattern {
                json!({ "pattern": p })
            } else {
                json!({})
            };

            let resp = client
                .call_tool("list", args)
                .await
                .context("qmd list")?;

            // Try structured response.
            if let Some(files) = resp["files"].as_array() {
                let parsed = files
                    .iter()
                    .map(|f| DocumentMeta {
                        path: f["path"].as_str().unwrap_or("").to_string(),
                        modified_at: f["modified_at"].as_str().unwrap_or("").to_string(),
                        size: f["size"].as_u64().unwrap_or(0),
                    })
                    .collect();
                return Ok(parsed);
            }

            // Fall back: parse newline-separated paths from text content.
            if let Some(text) = Self::extract_text(&resp) {
                let parsed = text
                    .lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| DocumentMeta {
                        path: l.trim().to_string(),
                        modified_at: String::new(),
                        size: 0,
                    })
                    .collect();
                return Ok(parsed);
            }

            Ok(vec![])
        })
    }
}
