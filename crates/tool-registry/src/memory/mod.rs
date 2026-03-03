//! Memory tool system for pince agents.
//!
//! Provides a `MemoryBackend` trait defining the communication interface,
//! a `QmdBackend` implementation backed by the `qmd` process, a
//! `MemoryManager` for lifecycle management, and four tool handlers:
//! `memory_search`, `memory_get`, `memory_store`, `memory_list`.
//!
//! # Lifecycle
//!
//! 1. Supervisor creates `MemoryManager::new(config)`.
//! 2. Supervisor calls `manager.start().await` (spawns the backend process).
//! 3. Supervisor registers tool handlers from `manager.tool_handlers()`.
//! 4. On shutdown, supervisor calls `manager.stop().await`.

pub mod manager;
pub mod qmd;
pub mod tools;
pub mod types;

pub use manager::MemoryManager;
pub use types::{
    BackendStatus, Document, DocumentMeta, MemoryConfig, SearchResult, StoreResult, StoreStatus,
};

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

/// Communication interface for a memory backend.
///
/// All methods take `&self` — implementations use interior mutability
/// (e.g. `tokio::sync::Mutex`) where mutable state is required, which allows
/// backends to be shared as `Arc<dyn MemoryBackend>` across tool handlers.
///
/// Uses boxed futures (same pattern as `ToolHandler`) to remain object-safe
/// without requiring `async_trait`.
pub trait MemoryBackend: Send + Sync {
    /// Health check — is the backend alive and responsive?
    fn health_check<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<BackendStatus>> + Send + 'a>>;

    /// Hybrid search across the memory store.
    fn search<'a>(
        &'a self,
        query: &'a str,
        collection: Option<&'a str>,
        limit: Option<u32>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchResult>>> + Send + 'a>>;

    /// Retrieve a document by path.
    fn get<'a>(
        &'a self,
        path: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Document>> + Send + 'a>>;

    /// Write or update a document.
    fn store<'a>(
        &'a self,
        path: &'a str,
        content: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<StoreResult>> + Send + 'a>>;

    /// List documents matching an optional glob pattern.
    fn list<'a>(
        &'a self,
        pattern: Option<&'a str>,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<DocumentMeta>>> + Send + 'a>>;
}
