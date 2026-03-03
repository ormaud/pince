//! `read_file` built-in tool.

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
        "read_file",
        "Read the contents of a file at the given path.",
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file to read."
                }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
        RiskLevel::Safe,
    )
}

pub struct ReadFileHandler {
    pub protected: Arc<ProtectedPaths>,
}

impl ToolHandler for ReadFileHandler {
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

            let contents = tokio::fs::read_to_string(path).await.map_err(|e| {
                ToolError::ExecutionFailed(anyhow::anyhow!("read_file '{}': {e}", path.display()))
            })?;

            Ok(json!({ "contents": contents }))
        })
    }
}
