//! `list_files` built-in tool.

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
        "list_files",
        "List the files and directories in a given directory.",
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the directory to list."
                }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
        RiskLevel::Safe,
    )
}

pub struct ListFilesHandler {
    pub protected: Arc<ProtectedPaths>,
}

impl ToolHandler for ListFilesHandler {
    fn execute<'a>(
        &'a self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + 'a>> {
        Box::pin(async move {
            let path_str = args["path"]
                .as_str()
                .ok_or_else(|| ToolError::InvalidArguments("missing 'path'".into()))?;
            let path = std::path::Path::new(path_str);

            if self.protected.is_protected(path) {
                return Err(self.protected.deny(path));
            }

            let mut dir = tokio::fs::read_dir(path).await.map_err(|e| {
                ToolError::ExecutionFailed(anyhow::anyhow!("list_files '{}': {e}", path.display()))
            })?;

            let mut entries = Vec::new();
            while let Some(entry) = dir.next_entry().await.map_err(|e| {
                ToolError::ExecutionFailed(anyhow::anyhow!("list_files read entry: {e}"))
            })? {
                let file_name = entry.file_name().to_string_lossy().to_string();
                let metadata = entry.metadata().await.ok();
                let is_dir = metadata.map(|m| m.is_dir()).unwrap_or(false);
                entries.push(json!({
                    "name": file_name,
                    "is_dir": is_dir
                }));
            }

            // Sort for deterministic output.
            entries.sort_by(|a, b| {
                let a_name = a["name"].as_str().unwrap_or("");
                let b_name = b["name"].as_str().unwrap_or("");
                a_name.cmp(b_name)
            });

            Ok(json!({ "entries": entries }))
        })
    }
}
