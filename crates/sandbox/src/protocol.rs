//! Host ↔ guest communication protocol.
//!
//! Wire format: 4-byte little-endian length prefix followed by a JSON payload.
//!
//! Request (host → guest):
//!   {"tool": "read_file", "args": {"path": "foo.txt"}}
//!
//! Response (guest → host):
//!   {"ok": true, "result": {...}}
//!   {"ok": false, "error": "file not found"}
//!
//! Shutdown (host → guest):
//!   {"shutdown": true}

use serde::{Deserialize, Serialize};

/// Maximum allowed payload size (64 MiB).
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

/// Tool execution request from host to guest.
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolRequest {
    pub tool: String,
    pub args: serde_json::Value,
}

/// Tool execution response from guest to host.
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ToolResponse {
    pub fn success(result: serde_json::Value) -> Self {
        Self { ok: true, result: Some(result), error: None }
    }

    pub fn failure(error: impl Into<String>) -> Self {
        Self { ok: false, result: None, error: Some(error.into()) }
    }
}

/// Graceful shutdown command from host to guest.
#[derive(Debug, Serialize, Deserialize)]
pub struct ShutdownRequest {
    pub shutdown: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_request_roundtrip() {
        let req = ToolRequest {
            tool: "read_file".to_string(),
            args: serde_json::json!({"path": "foo.txt"}),
        };
        let json = serde_json::to_string(&req).unwrap();
        let decoded: ToolRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.tool, "read_file");
    }

    #[test]
    fn tool_response_success_omits_error_field() {
        let resp = ToolResponse::success(serde_json::json!({"content": "hello"}));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("error"));
        assert!(json.contains("result"));
    }

    #[test]
    fn tool_response_failure_omits_result_field() {
        let resp = ToolResponse::failure("file not found");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("\"result\""));
        assert!(json.contains("error"));
    }
}
