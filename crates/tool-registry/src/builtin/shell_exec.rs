//! `shell_exec` built-in tool.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::process::Command;

use crate::{
    ToolError, ToolHandler, ToolOutput,
    schema::{RiskLevel, ToolSchema},
};

use super::ProtectedPaths;

pub fn schema() -> ToolSchema {
    ToolSchema::new(
        "shell_exec",
        "Execute a shell command and return its stdout, stderr, and exit code.",
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute."
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory for the command (optional)."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 30, max: 300).",
                    "minimum": 1,
                    "maximum": 300
                }
            },
            "required": ["command"],
            "additionalProperties": false
        }),
        RiskLevel::Dangerous,
    )
}

pub struct ShellExecHandler {
    pub protected: Arc<ProtectedPaths>,
}

impl ToolHandler for ShellExecHandler {
    fn execute<'a>(
        &'a self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + 'a>> {
        Box::pin(async move {
            let command = args["command"]
                .as_str()
                .ok_or_else(|| ToolError::InvalidArguments("missing 'command'".into()))?;

            // Heuristic check: reject commands that reference protected paths.
            self.check_command_for_protected_paths(command)?;

            let cwd = args["cwd"].as_str().map(String::from);
            let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(30).min(300);

            let mut cmd = Command::new("sh");
            cmd.arg("-c").arg(command);
            cmd.kill_on_drop(true);

            if let Some(ref dir) = cwd {
                cmd.current_dir(dir);
            }

            let output = tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                cmd.output(),
            )
            .await
            .map_err(|_| {
                ToolError::ExecutionFailed(anyhow::anyhow!(
                    "shell_exec timed out after {timeout_secs}s"
                ))
            })?
            .map_err(|e| ToolError::ExecutionFailed(anyhow::anyhow!("shell_exec spawn: {e}")))?;

            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let exit_code = output.status.code().unwrap_or(-1);

            Ok(json!({
                "stdout": stdout,
                "stderr": stderr,
                "exit_code": exit_code
            }))
        })
    }
}

impl ShellExecHandler {
    /// Reject commands that contain references to protected paths.
    ///
    /// This is a best-effort heuristic; it can't catch all cases (e.g. variables),
    /// but it prevents straightforward leakage.
    fn check_command_for_protected_paths(&self, command: &str) -> Result<(), ToolError> {
        for path in &self.protected.paths {
            let path_str = path.to_string_lossy();
            if command.contains(path_str.as_ref()) {
                return Err(ToolError::AccessDenied(format!(
                    "command references protected path '{}'",
                    path.display()
                )));
            }
        }
        Ok(())
    }
}
