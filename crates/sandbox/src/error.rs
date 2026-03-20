//! Sandbox error types.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SandboxError {
    #[error("failed to spawn sandbox for agent '{0}': {1}")]
    SpawnFailed(String, String),

    #[error("tool execution failed: {0}")]
    ExecutionFailed(String),

    #[error("sandbox operation timed out")]
    Timeout,

    #[error("no sandbox found for agent '{0}'")]
    NotFound(String),

    #[error("vsock communication error: {0}")]
    Communication(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
