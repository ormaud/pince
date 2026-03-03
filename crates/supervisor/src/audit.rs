//! Append-only JSONL audit log for tool call decisions.

use std::path::PathBuf;

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use tokio::{
    fs::OpenOptions,
    io::AsyncWriteExt,
    sync::Mutex,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Serialize)]
pub struct AuditEntry {
    pub timestamp: String,
    pub agent_id: String,
    pub tool_name: String,
    pub arguments_summary: String,
    pub decision: Decision,
    pub result_summary: String,
}

pub struct AuditLog {
    path: PathBuf,
    // Mutex ensures we don't interleave partial writes.
    _lock: Mutex<()>,
}

impl AuditLog {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            _lock: Mutex::new(()),
        }
    }

    pub async fn append(&self, entry: AuditEntry) -> Result<()> {
        let _guard = self._lock.lock().await;

        // Ensure parent directory exists.
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut line = serde_json::to_string(&entry)?;
        line.push('\n');

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }
}

impl AuditEntry {
    pub fn new(
        agent_id: impl Into<String>,
        tool_name: impl Into<String>,
        arguments_summary: impl Into<String>,
        decision: Decision,
        result_summary: impl Into<String>,
    ) -> Self {
        Self {
            timestamp: Utc::now().to_rfc3339(),
            agent_id: agent_id.into(),
            tool_name: tool_name.into(),
            arguments_summary: arguments_summary.into(),
            decision,
            result_summary: result_summary.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn writes_jsonl_entries() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("audit.jsonl");
        let log = AuditLog::new(path.clone());

        let agent_id = "test-agent-1";
        log.append(AuditEntry::new(
            agent_id,
            "read_file",
            r#"{"path":"/etc/passwd"}"#,
            Decision::Deny,
            "permission denied",
        ))
        .await
        .unwrap();

        log.append(AuditEntry::new(
            "test-agent-1",
            "list_dir",
            r#"{"path":"/tmp"}"#,
            Decision::Allow,
            "ok",
        ))
        .await
        .unwrap();

        let contents = tokio::fs::read_to_string(path.as_path()).await.unwrap();
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 2);

        // Each line must be valid JSON.
        for line in &lines {
            let val: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(val.get("timestamp").is_some());
            assert!(val.get("tool_name").is_some());
        }
    }
}
