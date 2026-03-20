//! `SandboxRunner` — a unified enum over real and mock sandbox backends.
//!
//! Use `SandboxRunner::Mock` in CI/tests (no `/dev/kvm` required) and
//! `SandboxRunner::Real` in production.

use std::path::PathBuf;

use serde_json::Value;

use crate::config::SandboxConfig;
use crate::error::SandboxError;
use crate::manager::SandboxManager;
use crate::mock::MockSandboxManager;

/// A unified sandbox backend that can dispatch to either the real Firecracker
/// `SandboxManager` or the in-process `MockSandboxManager`.
pub enum SandboxRunner {
    /// Real Firecracker microVM backend (requires `/dev/kvm`).
    Real(SandboxManager),
    /// In-process mock backend for CI and tests.
    Mock(MockSandboxManager),
}

impl SandboxRunner {
    /// Create a new real Firecracker backend.
    pub fn real(config: SandboxConfig, run_dir: PathBuf) -> Self {
        Self::Real(SandboxManager::new(config, run_dir))
    }

    /// Create a new mock backend.
    pub fn mock(config: &SandboxConfig) -> Self {
        Self::Mock(MockSandboxManager::new(config))
    }

    /// Spawn a sandbox for the given agent.
    pub async fn spawn(&mut self, agent_id: &str) -> Result<(), SandboxError> {
        match self {
            Self::Real(m) => m.spawn(agent_id).await,
            Self::Mock(m) => m.spawn(agent_id).await,
        }
    }

    /// Execute a tool inside the agent's sandbox.
    pub async fn execute(
        &self,
        agent_id: &str,
        tool: &str,
        args: Value,
    ) -> Result<Value, SandboxError> {
        match self {
            Self::Real(m) => m.execute(agent_id, tool, args).await,
            Self::Mock(m) => m.execute(agent_id, tool, args).await,
        }
    }

    /// Destroy the agent's sandbox.
    pub async fn destroy(
        &mut self,
        agent_id: &str,
        cleanup_workspace: bool,
    ) -> Result<(), SandboxError> {
        match self {
            Self::Real(m) => m.destroy(agent_id, cleanup_workspace).await,
            Self::Mock(m) => m.destroy(agent_id, cleanup_workspace).await,
        }
    }

    /// Returns `true` if a sandbox exists for the given agent.
    pub fn has_sandbox(&self, agent_id: &str) -> bool {
        match self {
            Self::Real(m) => m.has_sandbox(agent_id),
            Self::Mock(m) => m.has_sandbox(agent_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_runner() -> (SandboxRunner, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cfg = SandboxConfig {
            workspace_base: dir.path().join("workspaces"),
            ..SandboxConfig::default()
        };
        (SandboxRunner::mock(&cfg), dir)
    }

    #[tokio::test]
    async fn mock_runner_spawn_and_execute() {
        let (mut runner, _dir) = mock_runner();
        runner.spawn("agent-1").await.unwrap();
        assert!(runner.has_sandbox("agent-1"));

        let result = runner
            .execute(
                "agent-1",
                "write_file",
                serde_json::json!({"path": "test.txt", "content": "hello"}),
            )
            .await
            .unwrap();
        assert_eq!(result["written"], 5);
    }

    #[tokio::test]
    async fn mock_runner_destroy() {
        let (mut runner, _dir) = mock_runner();
        runner.spawn("agent-1").await.unwrap();
        runner.destroy("agent-1", false).await.unwrap();
        assert!(!runner.has_sandbox("agent-1"));
    }
}
