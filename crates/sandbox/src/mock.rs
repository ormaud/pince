//! Mock sandbox for CI environments without `/dev/kvm`.
//!
//! Executes tools directly on the host filesystem inside the agent's workspace
//! directory. Provides the same API surface as `SandboxManager` so the
//! supervisor can use either interchangeably at compile time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::config::SandboxConfig;
use crate::error::SandboxError;

/// In-process tool executor for CI/testing environments.
pub struct MockSandboxManager {
    workspace_base: PathBuf,
    sandboxes: HashMap<String, PathBuf>,
}

impl MockSandboxManager {
    pub fn new(config: &SandboxConfig) -> Self {
        Self {
            workspace_base: config.workspace_base.clone(),
            sandboxes: HashMap::new(),
        }
    }

    /// Create the workspace directory for the agent.
    pub async fn spawn(&mut self, agent_id: &str) -> Result<(), SandboxError> {
        if self.sandboxes.contains_key(agent_id) {
            return Err(SandboxError::SpawnFailed(
                agent_id.to_string(),
                "mock sandbox already exists for this agent".to_string(),
            ));
        }
        let workspace = self.workspace_base.join(agent_id);
        std::fs::create_dir_all(&workspace).map_err(|e| {
            SandboxError::SpawnFailed(
                agent_id.to_string(),
                format!("create workspace dir: {e}"),
            )
        })?;
        self.sandboxes.insert(agent_id.to_string(), workspace);
        tracing::debug!(agent_id, "mock sandbox spawned");
        Ok(())
    }

    /// Execute a tool in-process, within the agent's workspace directory.
    pub async fn execute(
        &self,
        agent_id: &str,
        tool: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, SandboxError> {
        let workspace = self
            .sandboxes
            .get(agent_id)
            .ok_or_else(|| SandboxError::NotFound(agent_id.to_string()))?;

        dispatch_tool(workspace, tool, &args).await
    }

    /// Remove the sandbox (and optionally the workspace directory).
    pub async fn destroy(
        &mut self,
        agent_id: &str,
        cleanup_workspace: bool,
    ) -> Result<(), SandboxError> {
        let workspace = self
            .sandboxes
            .remove(agent_id)
            .ok_or_else(|| SandboxError::NotFound(agent_id.to_string()))?;

        if cleanup_workspace {
            let _ = tokio::fs::remove_dir_all(&workspace).await;
        }
        tracing::debug!(agent_id, "mock sandbox destroyed");
        Ok(())
    }

    /// Get the workspace path for an agent.
    pub fn workspace_path(&self, agent_id: &str) -> Option<&Path> {
        self.sandboxes.get(agent_id).map(PathBuf::as_path)
    }

    /// Returns `true` if a mock sandbox exists for the given agent.
    pub fn has_sandbox(&self, agent_id: &str) -> bool {
        self.sandboxes.contains_key(agent_id)
    }
}

// ── Tool dispatcher ───────────────────────────────────────────────────────────

async fn dispatch_tool(
    workspace: &Path,
    tool: &str,
    args: &serde_json::Value,
) -> Result<serde_json::Value, SandboxError> {
    match tool {
        "read_file" => tool_read_file(workspace, args).await,
        "write_file" => tool_write_file(workspace, args).await,
        "list_dir" => tool_list_dir(workspace, args).await,
        "delete_file" => tool_delete_file(workspace, args).await,
        "shell_exec" => tool_shell_exec(workspace, args).await,
        other => Err(SandboxError::ExecutionFailed(format!("unknown tool: {other}"))),
    }
}

/// Resolve a relative path safely within the workspace, rejecting traversals.
fn resolve_path(workspace: &Path, relative: &str) -> Result<PathBuf, SandboxError> {
    // Strip any leading `/` to treat all paths as relative to workspace.
    let stripped = relative.trim_start_matches('/');
    let candidate = workspace.join(stripped);

    // Canonicalize the workspace so we can compare prefixes.
    let ws_canonical = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());

    // Normalize the candidate without requiring it to exist.
    let normalized = normalize_path(&candidate);

    if normalized.starts_with(&ws_canonical) || normalized.starts_with(workspace) {
        Ok(normalized)
    } else {
        Err(SandboxError::ExecutionFailed(format!(
            "path traversal rejected: {relative}"
        )))
    }
}

/// Normalize a path without calling canonicalize (which requires the path to exist).
fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

// ── Tool implementations ──────────────────────────────────────────────────────

async fn tool_read_file(
    workspace: &Path,
    args: &serde_json::Value,
) -> Result<serde_json::Value, SandboxError> {
    let path_str = args["path"]
        .as_str()
        .ok_or_else(|| SandboxError::ExecutionFailed("read_file: missing 'path'".into()))?;

    let abs = resolve_path(workspace, path_str)?;
    let content = tokio::fs::read_to_string(&abs).await.map_err(|e| {
        SandboxError::ExecutionFailed(format!("read_file {path_str}: {e}"))
    })?;

    Ok(serde_json::json!({ "content": content }))
}

async fn tool_write_file(
    workspace: &Path,
    args: &serde_json::Value,
) -> Result<serde_json::Value, SandboxError> {
    let path_str = args["path"]
        .as_str()
        .ok_or_else(|| SandboxError::ExecutionFailed("write_file: missing 'path'".into()))?;
    let content = args["content"]
        .as_str()
        .ok_or_else(|| SandboxError::ExecutionFailed("write_file: missing 'content'".into()))?;

    let abs = resolve_path(workspace, path_str)?;

    if let Some(parent) = abs.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| {
            SandboxError::ExecutionFailed(format!("write_file create dirs: {e}"))
        })?;
    }

    tokio::fs::write(&abs, content).await.map_err(|e| {
        SandboxError::ExecutionFailed(format!("write_file {path_str}: {e}"))
    })?;

    Ok(serde_json::json!({ "written": content.len() }))
}

async fn tool_list_dir(
    workspace: &Path,
    args: &serde_json::Value,
) -> Result<serde_json::Value, SandboxError> {
    let path_str = args["path"].as_str().unwrap_or(".");
    let abs = resolve_path(workspace, path_str)?;

    let mut entries = tokio::fs::read_dir(&abs).await.map_err(|e| {
        SandboxError::ExecutionFailed(format!("list_dir {path_str}: {e}"))
    })?;

    let mut names: Vec<serde_json::Value> = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| {
        SandboxError::ExecutionFailed(format!("list_dir read entry: {e}"))
    })? {
        let meta = entry.metadata().await.ok();
        let is_dir = meta.map(|m| m.is_dir()).unwrap_or(false);
        names.push(serde_json::json!({
            "name": entry.file_name().to_string_lossy(),
            "is_dir": is_dir,
        }));
    }

    Ok(serde_json::json!({ "entries": names }))
}

async fn tool_delete_file(
    workspace: &Path,
    args: &serde_json::Value,
) -> Result<serde_json::Value, SandboxError> {
    let path_str = args["path"]
        .as_str()
        .ok_or_else(|| SandboxError::ExecutionFailed("delete_file: missing 'path'".into()))?;

    let abs = resolve_path(workspace, path_str)?;
    let meta = tokio::fs::metadata(&abs).await.map_err(|e| {
        SandboxError::ExecutionFailed(format!("delete_file stat {path_str}: {e}"))
    })?;

    if meta.is_dir() {
        tokio::fs::remove_dir_all(&abs).await.map_err(|e| {
            SandboxError::ExecutionFailed(format!("delete_file (dir) {path_str}: {e}"))
        })?;
    } else {
        tokio::fs::remove_file(&abs).await.map_err(|e| {
            SandboxError::ExecutionFailed(format!("delete_file {path_str}: {e}"))
        })?;
    }

    Ok(serde_json::json!({ "deleted": path_str }))
}

async fn tool_shell_exec(
    workspace: &Path,
    args: &serde_json::Value,
) -> Result<serde_json::Value, SandboxError> {
    let command = args["command"]
        .as_str()
        .ok_or_else(|| SandboxError::ExecutionFailed("shell_exec: missing 'command'".into()))?;

    let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(30);

    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        tokio::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(workspace)
            .stdin(Stdio::null())
            .output(),
    )
    .await
    .map_err(|_| SandboxError::Timeout)?
    .map_err(|e| SandboxError::ExecutionFailed(format!("shell_exec: {e}")))?;

    Ok(serde_json::json!({
        "exit_code": output.status.code().unwrap_or(-1),
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
    }))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_mgr(dir: &tempfile::TempDir) -> MockSandboxManager {
        let cfg = SandboxConfig {
            workspace_base: dir.path().join("workspaces"),
            ..SandboxConfig::default()
        };
        MockSandboxManager::new(&cfg)
    }

    #[tokio::test]
    async fn spawn_creates_workspace_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = mock_mgr(&dir);
        mgr.spawn("agent-1").await.unwrap();
        assert!(dir.path().join("workspaces/agent-1").is_dir());
    }

    #[tokio::test]
    async fn double_spawn_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = mock_mgr(&dir);
        mgr.spawn("agent-1").await.unwrap();
        assert!(mgr.spawn("agent-1").await.is_err());
    }

    #[tokio::test]
    async fn write_then_read_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = mock_mgr(&dir);
        mgr.spawn("agent-1").await.unwrap();

        mgr.execute(
            "agent-1",
            "write_file",
            serde_json::json!({"path": "hello.txt", "content": "Hello, world!"}),
        )
        .await
        .unwrap();

        let result = mgr
            .execute("agent-1", "read_file", serde_json::json!({"path": "hello.txt"}))
            .await
            .unwrap();
        assert_eq!(result["content"], "Hello, world!");
    }

    #[tokio::test]
    async fn list_dir_returns_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = mock_mgr(&dir);
        mgr.spawn("agent-1").await.unwrap();

        mgr.execute(
            "agent-1",
            "write_file",
            serde_json::json!({"path": "a.txt", "content": "a"}),
        )
        .await
        .unwrap();
        mgr.execute(
            "agent-1",
            "write_file",
            serde_json::json!({"path": "b.txt", "content": "b"}),
        )
        .await
        .unwrap();

        let result = mgr
            .execute("agent-1", "list_dir", serde_json::json!({"path": "."}))
            .await
            .unwrap();
        let entries = result["entries"].as_array().unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"b.txt"));
    }

    #[tokio::test]
    async fn delete_file_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = mock_mgr(&dir);
        mgr.spawn("agent-1").await.unwrap();

        mgr.execute(
            "agent-1",
            "write_file",
            serde_json::json!({"path": "to_delete.txt", "content": "bye"}),
        )
        .await
        .unwrap();

        mgr.execute(
            "agent-1",
            "delete_file",
            serde_json::json!({"path": "to_delete.txt"}),
        )
        .await
        .unwrap();

        assert!(mgr
            .execute("agent-1", "read_file", serde_json::json!({"path": "to_delete.txt"}))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn shell_exec_runs_command() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = mock_mgr(&dir);
        mgr.spawn("agent-1").await.unwrap();

        let result = mgr
            .execute(
                "agent-1",
                "shell_exec",
                serde_json::json!({"command": "echo hello"}),
            )
            .await
            .unwrap();
        assert_eq!(result["exit_code"], 0);
        assert_eq!(result["stdout"].as_str().unwrap().trim(), "hello");
    }

    #[tokio::test]
    async fn shell_exec_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = mock_mgr(&dir);
        mgr.spawn("agent-1").await.unwrap();

        let result = mgr
            .execute("agent-1", "shell_exec", serde_json::json!({"command": "exit 42"}))
            .await
            .unwrap();
        assert_eq!(result["exit_code"], 42);
    }

    #[tokio::test]
    async fn path_traversal_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = mock_mgr(&dir);
        mgr.spawn("agent-1").await.unwrap();

        let result = mgr
            .execute(
                "agent-1",
                "read_file",
                serde_json::json!({"path": "../../etc/passwd"}),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_fails_for_unknown_agent() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = mock_mgr(&dir);

        let result = mgr.execute("ghost", "read_file", serde_json::json!({"path": "x"})).await;
        assert!(matches!(result, Err(SandboxError::NotFound(_))));
    }

    #[tokio::test]
    async fn destroy_cleans_up_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = mock_mgr(&dir);
        mgr.spawn("agent-1").await.unwrap();
        let ws = dir.path().join("workspaces/agent-1");
        assert!(ws.is_dir());

        mgr.destroy("agent-1", true).await.unwrap();
        assert!(!ws.exists());
        assert!(!mgr.has_sandbox("agent-1"));
    }

    #[tokio::test]
    async fn destroy_without_cleanup_keeps_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = mock_mgr(&dir);
        mgr.spawn("agent-1").await.unwrap();
        let ws = dir.path().join("workspaces/agent-1");

        mgr.destroy("agent-1", false).await.unwrap();
        assert!(ws.is_dir());
    }

    #[tokio::test]
    async fn unknown_tool_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = mock_mgr(&dir);
        mgr.spawn("agent-1").await.unwrap();

        let result = mgr
            .execute("agent-1", "nonexistent_tool", serde_json::json!({}))
            .await;
        assert!(matches!(result, Err(SandboxError::ExecutionFailed(_))));
    }

    #[test]
    fn normalize_path_resolves_dotdot() {
        let p = PathBuf::from("/workspace/sub/../foo.txt");
        let normalized = normalize_path(&p);
        assert_eq!(normalized, PathBuf::from("/workspace/foo.txt"));
    }

    #[test]
    fn resolve_path_rejects_traversal() {
        let ws = PathBuf::from("/workspace");
        assert!(resolve_path(&ws, "../../etc/passwd").is_err());
        assert!(resolve_path(&ws, "../other_agent/secret").is_err());
    }

    #[test]
    fn resolve_path_accepts_valid_paths() {
        let ws = PathBuf::from("/workspace");
        assert!(resolve_path(&ws, "foo.txt").is_ok());
        assert!(resolve_path(&ws, "sub/dir/file.txt").is_ok());
        assert!(resolve_path(&ws, "/foo.txt").is_ok()); // leading slash stripped
    }
}
