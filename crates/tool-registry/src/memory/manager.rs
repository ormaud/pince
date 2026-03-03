//! `MemoryManager` — owns the memory backend lifecycle.
//!
//! The supervisor creates one `MemoryManager` at startup, calls `start()`,
//! then passes `manager.backend()` to the memory tool handlers.
//! On shutdown the supervisor calls `manager.stop().await`.

use std::sync::Arc;

use anyhow::{Context, Result};

use super::{
    MemoryBackend,
    qmd::QmdBackend,
    types::MemoryConfig,
};

/// Owns the memory backend and manages its lifecycle.
pub struct MemoryManager {
    config: MemoryConfig,
    /// The concrete backend for lifecycle management (shutdown).
    qmd: Option<Arc<QmdBackend>>,
    /// Trait-object view of the backend shared with tool handlers.
    shared: Option<Arc<dyn MemoryBackend>>,
}

impl MemoryManager {
    /// Create a new manager using the default `QmdBackend`.
    pub fn new(config: MemoryConfig) -> Self {
        Self { config, qmd: None, shared: None }
    }

    /// Create a manager with a custom backend for testing.
    ///
    /// `start()` is a no-op when using a custom backend.
    pub fn with_backend(config: MemoryConfig, backend: Arc<dyn MemoryBackend>) -> Self {
        Self { config, qmd: None, shared: Some(backend) }
    }

    /// Start the memory backend. Must be called before `backend()`.
    ///
    /// Spawns the qmd process and creates the memory store directories.
    /// Idempotent — safe to call multiple times (subsequent calls are no-ops).
    pub async fn start(&mut self) -> Result<()> {
        if self.shared.is_some() {
            return Ok(()); // already started or using custom backend
        }
        let backend = Arc::new(
            QmdBackend::spawn(&self.config)
                .await
                .context("start qmd memory backend")?,
        );
        self.shared = Some(Arc::clone(&backend) as Arc<dyn MemoryBackend>);
        self.qmd = Some(backend);
        Ok(())
    }

    /// Graceful shutdown. Closes the qmd process's stdin and waits for exit.
    pub async fn stop(&mut self) {
        // Drop the shared backend first (tool handlers may still hold refs).
        self.shared = None;
        // Graceful shutdown via QmdBackend.
        if let Some(qmd) = self.qmd.take() {
            qmd.shutdown().await;
        }
    }

    /// Return a shared reference to the running backend for use in tool handlers.
    ///
    /// Returns `None` if `start()` has not been called.
    pub fn backend(&self) -> Option<Arc<dyn MemoryBackend>> {
        self.shared.as_ref().map(Arc::clone)
    }
}
