//! Tool implementations for the guest agent.
//!
//! All paths are resolved relative to `/workspace` (the mounted workspace drive).
//! Path traversal outside the workspace is rejected.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use pince_sandbox::protocol::{ToolRequest, ToolResponse};

const WORKSPACE: &str = "/workspace";

/// Dispatch a tool request and return a response.
pub fn dispatch(req: &ToolRequest) -> ToolResponse {
    match execute(req) {
        Ok(result) => ToolResponse::success(result),
        Err(e) => ToolResponse::failure(e),
    }
}

fn execute(req: &ToolRequest) -> Result<serde_json::Value, String> {
    match req.tool.as_str() {
        "read_file" => tool_read_file(&req.args),
        "write_file" => tool_write_file(&req.args),
        "list_dir" => tool_list_dir(&req.args),
        "delete_file" => tool_delete_file(&req.args),
        "shell_exec" => tool_shell_exec(&req.args),
        other => Err(format!("unknown tool: {other}")),
    }
}

// ── Path resolution ───────────────────────────────────────────────────────────

fn workspace() -> PathBuf {
    PathBuf::from(WORKSPACE)
}

fn resolve(relative: &str) -> Result<PathBuf, String> {
    let stripped = relative.trim_start_matches('/');
    let candidate = workspace().join(stripped);
    let normalized = normalize(&candidate);

    if normalized.starts_with(workspace()) {
        Ok(normalized)
    } else {
        Err(format!("path traversal rejected: {relative}"))
    }
}

fn normalize(path: &Path) -> PathBuf {
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

fn tool_read_file(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let path_str = args["path"].as_str().ok_or("read_file: missing 'path'")?;
    let abs = resolve(path_str)?;

    let mut content = String::new();
    std::fs::File::open(&abs)
        .map_err(|e| format!("read_file {path_str}: {e}"))?
        .read_to_string(&mut content)
        .map_err(|e| format!("read_file read {path_str}: {e}"))?;

    Ok(serde_json::json!({ "content": content }))
}

fn tool_write_file(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let path_str = args["path"].as_str().ok_or("write_file: missing 'path'")?;
    let content = args["content"].as_str().ok_or("write_file: missing 'content'")?;

    let abs = resolve(path_str)?;
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("write_file create dirs: {e}"))?;
    }

    std::fs::File::create(&abs)
        .map_err(|e| format!("write_file create {path_str}: {e}"))?
        .write_all(content.as_bytes())
        .map_err(|e| format!("write_file write {path_str}: {e}"))?;

    Ok(serde_json::json!({ "written": content.len() }))
}

fn tool_list_dir(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let path_str = args["path"].as_str().unwrap_or(".");
    let abs = resolve(path_str)?;

    let entries = std::fs::read_dir(&abs)
        .map_err(|e| format!("list_dir {path_str}: {e}"))?;

    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("list_dir entry: {e}"))?;
        let is_dir = entry.metadata().map(|m| m.is_dir()).unwrap_or(false);
        names.push(serde_json::json!({
            "name": entry.file_name().to_string_lossy(),
            "is_dir": is_dir,
        }));
    }

    Ok(serde_json::json!({ "entries": names }))
}

fn tool_delete_file(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let path_str = args["path"].as_str().ok_or("delete_file: missing 'path'")?;
    let abs = resolve(path_str)?;

    let meta = std::fs::metadata(&abs)
        .map_err(|e| format!("delete_file stat {path_str}: {e}"))?;

    if meta.is_dir() {
        std::fs::remove_dir_all(&abs)
            .map_err(|e| format!("delete_file (dir) {path_str}: {e}"))?;
    } else {
        std::fs::remove_file(&abs)
            .map_err(|e| format!("delete_file {path_str}: {e}"))?;
    }

    Ok(serde_json::json!({ "deleted": path_str }))
}

fn tool_shell_exec(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let command = args["command"].as_str().ok_or("shell_exec: missing 'command'")?;

    let output = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir(workspace())
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("shell_exec: {e}"))?;

    Ok(serde_json::json!({
        "exit_code": output.status.code().unwrap_or(-1),
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
    }))
}
