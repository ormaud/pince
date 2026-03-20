//! `pince-sandbox` — Firecracker microVM sandbox runtime for agent tool execution.
//!
//! # Overview
//!
//! Each agent session gets a dedicated Firecracker microVM for tool execution.
//! The supervisor manages the microVM lifecycle and communicates with the guest
//! agent over a vsock connection to execute tools in isolation.
//!
//! # Components
//!
//! - [`SandboxManager`]: manages real Firecracker microVM lifecycles.
//! - [`MockSandboxManager`]: in-process tool execution for CI without `/dev/kvm`.
//! - [`SandboxConfig`]: configuration (loaded from `supervisor.toml`).
//! - [`SandboxError`]: error type.
//!
//! # Usage
//!
//! ```rust,no_run
//! use pince_sandbox::{SandboxConfig, MockSandboxManager};
//!
//! # #[tokio::main]
//! # async fn main() {
//! let config = SandboxConfig::default();
//! let mut mgr = MockSandboxManager::new(&config);
//!
//! mgr.spawn("agent-123").await.unwrap();
//!
//! let result = mgr.execute(
//!     "agent-123",
//!     "shell_exec",
//!     serde_json::json!({"command": "echo hello"}),
//! ).await.unwrap();
//!
//! assert_eq!(result["exit_code"], 0);
//!
//! mgr.destroy("agent-123", true).await.unwrap();
//! # }
//! ```

pub mod config;
pub mod error;
pub mod manager;
pub mod mock;
pub mod protocol;

// Internal modules (not part of public API).
pub(crate) mod firecracker;
pub(crate) mod vsock;

// Re-export the most commonly used types at crate root.
pub use config::SandboxConfig;
pub use error::SandboxError;
pub use manager::SandboxManager;
pub use mock::MockSandboxManager;
