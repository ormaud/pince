//! Integration tests for MCP tool loading.
//!
//! Uses a Python one-liner as a minimal MCP server mock.

use serde_json::json;
use tool_registry::{ToolRegistry, mcp::load_mcp_server, config::McpServerConfig};

/// A minimal MCP server implemented as a shell script / Python that:
/// - Handles `initialize` → returns capabilities
/// - Handles `notifications/initialized` → ignores
/// - Handles `tools/list` → returns one tool: `echo_tool`
/// - Handles `tools/call` for `echo_tool` → returns the input as-is
const MOCK_MCP_SERVER: &str = r#"
import sys, json

def respond(req_id, result):
    msg = json.dumps({"jsonrpc": "2.0", "id": req_id, "result": result})
    sys.stdout.write(msg + "\n")
    sys.stdout.flush()

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
    except Exception:
        continue
    method = req.get("method", "")
    req_id = req.get("id")
    if method == "initialize":
        respond(req_id, {"protocolVersion": "2024-11-05", "capabilities": {}, "serverInfo": {"name": "mock", "version": "0.1"}})
    elif method == "notifications/initialized":
        pass  # notification, no response
    elif method == "tools/list":
        respond(req_id, {"tools": [{"name": "echo_tool", "description": "Echoes input", "inputSchema": {"type": "object", "properties": {"message": {"type": "string"}}, "required": ["message"]}}]})
    elif method == "tools/call":
        params = req.get("params", {})
        args = params.get("arguments", {})
        respond(req_id, {"content": [{"type": "text", "text": args.get("message", "")}]})
"#;

/// Spawn a minimal MCP server using Python.
async fn spawn_mock_mcp() -> McpServerConfig {
    // Write the script to a temp file.
    let dir = tempfile::TempDir::new().unwrap();
    let script_path = dir.path().join("mock_mcp.py");
    std::fs::write(&script_path, MOCK_MCP_SERVER).unwrap();

    // We need the tempdir to live as long as the test.
    // Leak it for the test duration (acceptable in tests).
    std::mem::forget(dir);

    McpServerConfig {
        name: "mock".into(),
        command: "python3".into(),
        args: vec![script_path.to_str().unwrap().to_string()],
        env: Default::default(),
    }
}

#[tokio::test]
async fn mcp_tool_loading() {
    // Skip if python3 is not available.
    if std::process::Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 not available, skipping MCP test");
        return;
    }

    let config = spawn_mock_mcp().await;
    let mut registry = ToolRegistry::new();

    load_mcp_server(&mut registry, &config)
        .await
        .expect("MCP server should load");

    assert!(registry.contains("echo_tool"), "echo_tool should be registered");
    let schemas = registry.schemas();
    let echo = schemas.iter().find(|s| s.name == "echo_tool").unwrap();
    assert!(echo.description.contains("Echoes"));
}

#[tokio::test]
async fn mcp_tool_call() {
    if std::process::Command::new("python3").arg("--version").output().is_err() {
        eprintln!("python3 not available, skipping MCP test");
        return;
    }

    let config = spawn_mock_mcp().await;
    let mut registry = ToolRegistry::new();

    load_mcp_server(&mut registry, &config)
        .await
        .expect("MCP server should load");

    // The mock server echoes the message field back in a content array.
    // The execute result is whatever the MCP server returns.
    let result = registry
        .execute("echo_tool", json!({"message": "hello MCP"}))
        .await
        .expect("echo_tool should execute");

    // The mock returns {"content": [{"type": "text", "text": "hello MCP"}]}.
    let content = result["content"].as_array().unwrap();
    assert_eq!(content[0]["text"], "hello MCP");
}
