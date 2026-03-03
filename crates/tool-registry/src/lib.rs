//! Tool registry for the pince supervisor.
//!
//! Defines the tool abstraction, a central registry, JSON Schema validation,
//! built-in tool handlers, and MCP tool loading.

pub mod builtin;
pub mod config;
pub mod mcp;
pub mod memory;
pub mod registry;
pub mod schema;
pub mod validate;

pub use builtin::FeedbackConfig;
pub use builtin::register_feedback;
pub use config::ToolRegistryConfig;
pub use registry::{RegisteredTool, ToolRegistry};
pub use schema::{RiskLevel, ToolSchema};

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

/// The result of executing a tool call.
pub type ToolOutput = Value;

/// Error type for tool execution failures.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("tool not found: {0}")]
    NotFound(String),

    #[error("invalid arguments: {0}")]
    InvalidArguments(String),

    #[error("access denied: {0}")]
    AccessDenied(String),

    #[error("execution failed: {0}")]
    ExecutionFailed(#[from] anyhow::Error),
}

/// A handler that executes a tool call in the supervisor process.
///
/// We use a boxed future instead of `async-trait` to keep the dependency
/// footprint small while still being object-safe.
pub trait ToolHandler: Send + Sync {
    fn execute<'a>(
        &'a self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + 'a>>;
}
