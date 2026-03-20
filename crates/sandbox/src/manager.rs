//! SandboxManager: manages Firecracker microVM lifecycle per agent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::Mutex;

use crate::config::SandboxConfig;
use crate::error::SandboxError;
use crate::firecracker::{create_workspace_image, FirecrackerInstance};
use crate::protocol::ToolRequest;
use crate::vsock::VsockConnection;

// ── Internal sandbox state ────────────────────────────────────────────────────

struct SandboxEntry {
    workspace: PathBuf,
    /// Path to the ext4 workspace image file (tracked for potential future inspection).
    _workspace_image: PathBuf,
    vsock: Mutex<VsockConnection>,
    firecracker: Mutex<FirecrackerInstance>,
}

// ── SandboxManager ────────────────────────────────────────────────────────────

/// Manages the lifecycle of Firecracker microVMs for agent tool execution.
pub struct SandboxManager {
    config: SandboxConfig,
    /// Directory for runtime files: Firecracker configs, vsock sockets.
    run_dir: PathBuf,
    sandboxes: HashMap<String, SandboxEntry>,
}

impl SandboxManager {
    /// Create a new `SandboxManager`.
    ///
    /// `run_dir` is where per-agent Firecracker config JSON and vsock socket
    /// files are placed (e.g. `$XDG_RUNTIME_DIR/pince/sandboxes`).
    pub fn new(config: SandboxConfig, run_dir: PathBuf) -> Self {
        Self { config, run_dir, sandboxes: HashMap::new() }
    }

    /// Spawn a Firecracker microVM for the given agent.
    ///
    /// Creates the agent's workspace directory and ext4 image, writes the
    /// Firecracker config, spawns the process, and waits for the guest agent
    /// to signal readiness over vsock.
    pub async fn spawn(&mut self, agent_id: &str) -> Result<(), SandboxError> {
        if self.sandboxes.contains_key(agent_id) {
            return Err(SandboxError::SpawnFailed(
                agent_id.to_string(),
                "sandbox already exists for this agent".to_string(),
            ));
        }

        let workspace = self.config.workspace_base.join(agent_id);
        tokio::fs::create_dir_all(&workspace).await.map_err(|e| {
            SandboxError::SpawnFailed(agent_id.to_string(), format!("create workspace dir: {e}"))
        })?;

        let workspace_image = workspace.join("workspace.ext4");
        create_workspace_image(&workspace_image, self.config.workspace_size_mb).await?;

        tokio::fs::create_dir_all(&self.run_dir).await.map_err(|e| {
            SandboxError::SpawnFailed(agent_id.to_string(), format!("create run dir: {e}"))
        })?;

        let fc = FirecrackerInstance::spawn(
            &self.config,
            agent_id,
            &workspace_image,
            &self.run_dir,
        )
        .await?;

        let uds_path = fc.uds_path.clone();
        let boot_timeout = Duration::from_secs(self.config.boot_timeout_secs);
        let vsock_port = self.config.vsock_port;

        tracing::info!(agent_id, "waiting for guest agent to become ready");
        let vsock = VsockConnection::connect_with_retry(&uds_path, vsock_port, boot_timeout)
            .await
            .map_err(|_| {
                SandboxError::SpawnFailed(
                    agent_id.to_string(),
                    "guest agent did not become ready within the boot timeout".to_string(),
                )
            })?;

        self.sandboxes.insert(
            agent_id.to_string(),
            SandboxEntry {
                workspace,
                _workspace_image: workspace_image,
                vsock: Mutex::new(vsock),
                firecracker: Mutex::new(fc),
            },
        );

        tracing::info!(agent_id, "sandbox ready");
        Ok(())
    }

    /// Execute a tool inside the agent's sandbox.
    ///
    /// Sends a `ToolRequest` to the guest over vsock and returns the JSON result.
    pub async fn execute(
        &self,
        agent_id: &str,
        tool: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, SandboxError> {
        let entry = self
            .sandboxes
            .get(agent_id)
            .ok_or_else(|| SandboxError::NotFound(agent_id.to_string()))?;

        let req = ToolRequest { tool: tool.to_string(), args };
        let mut vsock = entry.vsock.lock().await;
        let resp = vsock.execute(&req).await?;

        if resp.ok {
            Ok(resp.result.unwrap_or(serde_json::Value::Null))
        } else {
            Err(SandboxError::ExecutionFailed(
                resp.error.unwrap_or_else(|| "unknown error".to_string()),
            ))
        }
    }

    /// Destroy the agent's sandbox.
    ///
    /// Sends a graceful shutdown to the guest, waits for Firecracker to exit
    /// (up to 5 seconds), then optionally removes the workspace directory.
    pub async fn destroy(
        &mut self,
        agent_id: &str,
        cleanup_workspace: bool,
    ) -> Result<(), SandboxError> {
        let entry = self
            .sandboxes
            .remove(agent_id)
            .ok_or_else(|| SandboxError::NotFound(agent_id.to_string()))?;

        // Try to send a graceful shutdown; ignore errors (guest may already be dead).
        if let Ok(mut vsock) = entry.vsock.try_lock() {
            let _ = vsock.send_shutdown().await;
        }

        // Shut down Firecracker with a 5-second timeout.
        let fc = entry.firecracker.into_inner();
        fc.shutdown(Duration::from_secs(5)).await;

        if cleanup_workspace {
            let _ = tokio::fs::remove_dir_all(&entry.workspace).await;
        }

        tracing::info!(agent_id, cleanup_workspace, "sandbox destroyed");
        Ok(())
    }

    /// Get the workspace path for an agent, if a sandbox exists.
    pub fn workspace_path(&self, agent_id: &str) -> Option<&Path> {
        self.sandboxes.get(agent_id).map(|e| e.workspace.as_path())
    }

    /// Returns `true` if a sandbox exists for the given agent.
    pub fn has_sandbox(&self, agent_id: &str) -> bool {
        self.sandboxes.contains_key(agent_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_config(workspace_base: PathBuf) -> SandboxConfig {
        SandboxConfig {
            workspace_base,
            ..SandboxConfig::default()
        }
    }

    #[test]
    fn workspace_path_returns_none_for_unknown_agent() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = make_config(dir.path().join("workspaces"));
        let mgr = SandboxManager::new(cfg, dir.path().join("run"));
        assert!(mgr.workspace_path("nonexistent").is_none());
    }

    #[test]
    fn has_sandbox_returns_false_initially() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = make_config(dir.path().join("workspaces"));
        let mgr = SandboxManager::new(cfg, dir.path().join("run"));
        assert!(!mgr.has_sandbox("agent-1"));
    }
}
