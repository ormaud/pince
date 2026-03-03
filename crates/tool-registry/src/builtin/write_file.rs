//! `write_file` built-in tool.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::{
    ToolError, ToolHandler, ToolOutput,
    schema::{RiskLevel, ToolSchema},
};

use super::ProtectedPaths;

pub fn schema() -> ToolSchema {
    ToolSchema::new(
        "write_file",
        "Write content to a file, creating it if it doesn't exist.",
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write."
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file."
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        }),
        RiskLevel::Sensitive,
    )
}

pub struct WriteFileHandler {
    pub protected: Arc<ProtectedPaths>,
}

impl ToolHandler for WriteFileHandler {
    fn execute<'a>(
        &'a self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + 'a>> {
        Box::pin(async move {
            let path_str = args["path"]
                .as_str()
                .ok_or_else(|| ToolError::InvalidArguments("missing 'path'".into()))?;
            let content = args["content"]
                .as_str()
                .ok_or_else(|| ToolError::InvalidArguments("missing 'content'".into()))?;
            let path = std::path::Path::new(path_str);

            if self.protected.is_protected(path) {
                return Err(self.protected.deny(path));
            }

            // Create parent directories if needed.
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    tokio::fs::create_dir_all(parent).await.map_err(|e| {
                        ToolError::ExecutionFailed(anyhow::anyhow!(
                            "write_file create dirs '{}': {e}",
                            parent.display()
                        ))
                    })?;
                }
            }

            tokio::fs::write(path, content).await.map_err(|e| {
                ToolError::ExecutionFailed(anyhow::anyhow!("write_file '{}': {e}", path.display()))
            })?;

            Ok(json!({ "written": content.len() }))
        })
    }
}
