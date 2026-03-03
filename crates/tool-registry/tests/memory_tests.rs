//! Tests for the memory tool system.
//!
//! Uses a mock `MemoryBackend` implementation to test the tool handlers and
//! path validation logic without requiring qmd to be installed.

use std::pin::Pin;
use std::future::Future;
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use serde_json::json;
use tool_registry::{
    ToolRegistry,
    memory::{
        MemoryBackend, MemoryManager,
        types::{
            BackendStatus, Document, DocumentMeta, MemoryConfig, SearchResult, StoreResult,
            StoreStatus,
        },
    },
};

// ── Mock backend ──────────────────────────────────────────────────────────────

#[derive(Default)]
struct MockMemory {
    /// path → content
    store: Mutex<std::collections::HashMap<String, String>>,
}

impl MockMemory {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl MemoryBackend for MockMemory {
    fn health_check<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<BackendStatus>> + Send + 'a>> {
        Box::pin(async { Ok(BackendStatus::Running) })
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        _collection: Option<&'a str>,
        limit: Option<u32>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchResult>>> + Send + 'a>> {
        let store = self.store.lock().unwrap().clone();
        let query = query.to_string();
        let limit = limit.unwrap_or(10) as usize;
        Box::pin(async move {
            let results: Vec<SearchResult> = store
                .iter()
                .filter(|(_, v)| v.contains(&query))
                .take(limit)
                .map(|(k, v)| SearchResult {
                    path: k.clone(),
                    snippet: v.chars().take(80).collect(),
                    score: 1.0,
                })
                .collect();
            Ok(results)
        })
    }

    fn get<'a>(
        &'a self,
        path: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Document>> + Send + 'a>> {
        let result = self
            .store
            .lock()
            .unwrap()
            .get(path)
            .cloned();
        let path = path.to_string();
        Box::pin(async move {
            match result {
                Some(content) => Ok(Document { path, content }),
                None => bail!("document not found: '{path}'"),
            }
        })
    }

    fn store<'a>(
        &'a self,
        path: &'a str,
        content: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<StoreResult>> + Send + 'a>> {
        let mut guard = self.store.lock().unwrap();
        let status = if guard.contains_key(path) {
            StoreStatus::Updated
        } else {
            StoreStatus::Created
        };
        guard.insert(path.to_string(), content.to_string());
        let path = path.to_string();
        Box::pin(async move { Ok(StoreResult { path, status }) })
    }

    fn list<'a>(
        &'a self,
        pattern: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<DocumentMeta>>> + Send + 'a>> {
        let store = self.store.lock().unwrap().clone();
        let pattern = pattern.map(|s| s.to_string());
        Box::pin(async move {
            let docs: Vec<DocumentMeta> = store
                .keys()
                .filter(|k| {
                    pattern
                        .as_deref()
                        .map(|p| k.starts_with(p.trim_end_matches('*').trim_end_matches('/')))
                        .unwrap_or(true)
                })
                .map(|k| DocumentMeta {
                    path: k.clone(),
                    modified_at: "2026-01-01T00:00:00Z".into(),
                    size: store[k].len() as u64,
                })
                .collect();
            Ok(docs)
        })
    }
}

// ── Helper: build a registry with mock backend ────────────────────────────────

fn make_registry_with_mock() -> (ToolRegistry, Arc<MockMemory>) {
    let mock = MockMemory::new();
    let backend: Arc<dyn MemoryBackend> = Arc::clone(&mock) as Arc<dyn MemoryBackend>;
    let mut registry = ToolRegistry::new();
    tool_registry::memory::tools::register_all(&mut registry, backend);
    (registry, mock)
}

// ── Tests: memory_store ───────────────────────────────────────────────────────

#[tokio::test]
async fn memory_store_creates_document() {
    let (registry, mock) = make_registry_with_mock();

    let result = registry
        .execute(
            "memory_store",
            json!({ "path": "knowledge/test.md", "content": "# Hello" }),
        )
        .await
        .expect("memory_store should succeed");

    assert_eq!(result["path"], "knowledge/test.md");
    assert_eq!(result["status"], "created");

    let guard = mock.store.lock().unwrap();
    assert_eq!(guard.get("knowledge/test.md").unwrap(), "# Hello");
}

#[tokio::test]
async fn memory_store_updates_existing_document() {
    let (registry, mock) = make_registry_with_mock();

    // First write.
    registry
        .execute(
            "memory_store",
            json!({ "path": "knowledge/test.md", "content": "v1" }),
        )
        .await
        .unwrap();

    // Second write.
    let result = registry
        .execute(
            "memory_store",
            json!({ "path": "knowledge/test.md", "content": "v2" }),
        )
        .await
        .expect("second store should succeed");

    assert_eq!(result["status"], "updated");
    assert_eq!(mock.store.lock().unwrap().get("knowledge/test.md").unwrap(), "v2");
}

// ── Tests: memory_get ─────────────────────────────────────────────────────────

#[tokio::test]
async fn memory_get_retrieves_stored_document() {
    let (registry, _mock) = make_registry_with_mock();

    registry
        .execute(
            "memory_store",
            json!({ "path": "tasks/todo.md", "content": "- [ ] item 1" }),
        )
        .await
        .unwrap();

    let result = registry
        .execute("memory_get", json!({ "path": "tasks/todo.md" }))
        .await
        .expect("memory_get should succeed");

    assert_eq!(result["path"], "tasks/todo.md");
    assert_eq!(result["content"], "- [ ] item 1");
}

#[tokio::test]
async fn memory_get_returns_error_for_missing_document() {
    let (registry, _mock) = make_registry_with_mock();

    let err = registry
        .execute("memory_get", json!({ "path": "does/not/exist.md" }))
        .await;

    assert!(err.is_err(), "memory_get on missing doc should fail");
}

// ── Tests: memory_search ──────────────────────────────────────────────────────

#[tokio::test]
async fn memory_search_finds_matching_documents() {
    let (registry, _mock) = make_registry_with_mock();

    registry
        .execute(
            "memory_store",
            json!({ "path": "knowledge/rust.md", "content": "Rust ownership model explained." }),
        )
        .await
        .unwrap();

    registry
        .execute(
            "memory_store",
            json!({ "path": "knowledge/python.md", "content": "Python is dynamically typed." }),
        )
        .await
        .unwrap();

    let result = registry
        .execute("memory_search", json!({ "query": "ownership" }))
        .await
        .expect("memory_search should succeed");

    let results = result["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["path"], "knowledge/rust.md");
}

#[tokio::test]
async fn memory_search_returns_empty_for_no_match() {
    let (registry, _mock) = make_registry_with_mock();

    registry
        .execute(
            "memory_store",
            json!({ "path": "knowledge/rust.md", "content": "Rust ownership model." }),
        )
        .await
        .unwrap();

    let result = registry
        .execute("memory_search", json!({ "query": "javascript" }))
        .await
        .expect("memory_search should succeed");

    let results = result["results"].as_array().unwrap();
    assert!(results.is_empty());
}

// ── Tests: memory_list ────────────────────────────────────────────────────────

#[tokio::test]
async fn memory_list_returns_all_documents() {
    let (registry, _mock) = make_registry_with_mock();

    for path in &["a.md", "b.md", "c.md"] {
        registry
            .execute(
                "memory_store",
                json!({ "path": path, "content": "content" }),
            )
            .await
            .unwrap();
    }

    let result = registry
        .execute("memory_list", json!({}))
        .await
        .expect("memory_list should succeed");

    let docs = result["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 3);
}

// ── Tests: path validation ────────────────────────────────────────────────────

#[tokio::test]
async fn path_traversal_is_rejected() {
    let (registry, _mock) = make_registry_with_mock();

    let err = registry
        .execute(
            "memory_store",
            json!({ "path": "../etc/passwd", "content": "evil" }),
        )
        .await;

    assert!(err.is_err(), "path traversal should be rejected");
    let msg = format!("{:?}", err.unwrap_err());
    assert!(
        msg.contains("traversal") || msg.contains("argument") || msg.contains("not allowed"),
        "error should mention path traversal: {msg}"
    );
}

#[tokio::test]
async fn absolute_path_is_rejected() {
    let (registry, _mock) = make_registry_with_mock();

    let err = registry
        .execute(
            "memory_get",
            json!({ "path": "/etc/passwd" }),
        )
        .await;

    assert!(err.is_err(), "absolute path should be rejected");
}

#[tokio::test]
async fn empty_path_is_rejected() {
    let (registry, _mock) = make_registry_with_mock();

    let err = registry
        .execute("memory_get", json!({ "path": "" }))
        .await;

    assert!(err.is_err(), "empty path should be rejected");
}

// ── Tests: MemoryManager ──────────────────────────────────────────────────────

#[tokio::test]
async fn memory_manager_with_backend() {
    let mock = MockMemory::new();
    let backend: Arc<dyn MemoryBackend> = Arc::clone(&mock) as Arc<dyn MemoryBackend>;
    let config = MemoryConfig::default_config();
    let manager = MemoryManager::with_backend(config, backend);

    // start() should be a no-op (custom backend already set)
    // backend() should return Some.
    let b = manager.backend().expect("backend should be set");
    assert_eq!(b.health_check().await.unwrap(), BackendStatus::Running);
}

// ── Tests: MemoryBackendConfig ────────────────────────────────────────────────

#[test]
fn memory_backend_config_defaults() {
    use tool_registry::config::MemoryBackendConfig;
    let config: MemoryBackendConfig = toml::from_str("").unwrap();
    assert_eq!(config.command, "qmd");
    assert_eq!(config.args, vec!["mcp"]);
    assert!(config.store_path.is_none());
}

#[test]
fn memory_backend_config_custom() {
    use tool_registry::config::MemoryBackendConfig;
    let config: MemoryBackendConfig = toml::from_str(
        r#"command = "custom-qmd"
args = ["serve", "--mcp"]
"#,
    )
    .unwrap();
    assert_eq!(config.command, "custom-qmd");
    assert_eq!(config.args, vec!["serve", "--mcp"]);
}
