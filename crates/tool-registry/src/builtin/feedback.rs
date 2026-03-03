//! `feedback` built-in tool.
//!
//! Submits user feedback to the Trame API. The supervisor resolves the API key
//! from the secret store; agents never see it.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Value, json};

use crate::{
    ToolError, ToolHandler, ToolOutput,
    schema::{RiskLevel, ToolSchema},
};

use secrets::SecretStore;

pub fn schema() -> ToolSchema {
    ToolSchema::new(
        "feedback",
        "Submit user feedback to the Trame project management system. \
         Requires user approval before submitting.",
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The feedback text to submit."
                },
                "project_id": {
                    "type": "string",
                    "description": "The Trame project UUID to submit feedback to."
                }
            },
            "required": ["content", "project_id"],
            "additionalProperties": false
        }),
        RiskLevel::Sensitive,
    )
}

/// Configuration for the feedback tool.
pub struct FeedbackConfig {
    /// Base URL of the Trame API (e.g. `https://trame.example.com`).
    pub api_base_url: String,
    /// Secret store used to resolve the `trame-api-key` secret.
    pub secret_store: Arc<SecretStore>,
}

pub struct FeedbackHandler {
    pub config: FeedbackConfig,
}

impl ToolHandler for FeedbackHandler {
    fn execute<'a>(
        &'a self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + 'a>> {
        Box::pin(async move {
            let content = args["content"]
                .as_str()
                .ok_or_else(|| ToolError::InvalidArguments("missing 'content'".into()))?
                .to_owned();

            let project_id = args["project_id"]
                .as_str()
                .ok_or_else(|| ToolError::InvalidArguments("missing 'project_id'".into()))?
                .to_owned();

            // Resolve API key from secret store (never exposed to agents).
            let api_key = self
                .config
                .secret_store
                .resolve("trame-api-key")
                .map_err(|e| {
                    ToolError::ExecutionFailed(anyhow::anyhow!("API key not found: {e}"))
                })?;
            let api_key_str = api_key
                .expose_str()
                .map_err(|e| {
                    ToolError::ExecutionFailed(anyhow::anyhow!("API key encoding error: {e}"))
                })?
                .to_owned();
            // Drop the SecretValue before the await point to avoid holding it across yields.
            drop(api_key);

            let url = format!(
                "{}/api/feedback",
                self.config.api_base_url.trim_end_matches('/')
            );

            let client = reqwest::Client::new();
            let response = client
                .post(&url)
                .bearer_auth(&api_key_str)
                .json(&json!({
                    "content": content,
                    "project_id": project_id
                }))
                .send()
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(anyhow::anyhow!("HTTP request failed: {e}"))
                })?;

            let status = response.status();
            if status.is_success() {
                Ok(json!({
                    "status": "submitted",
                    "message": "Feedback submitted successfully."
                }))
            } else {
                let body = response.text().await.unwrap_or_default();
                Err(ToolError::ExecutionFailed(anyhow::anyhow!(
                    "Trame API returned {status}: {body}"
                )))
            }
        })
    }
}
