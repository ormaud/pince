//! Integration tests for built-in tool handlers.

use serde_json::json;
use tempfile::TempDir;
use tool_registry::{
    ToolRegistry,
    builtin::{ProtectedPaths, register_all},
};

fn make_registry() -> (ToolRegistry, TempDir) {
    let dir = TempDir::new().unwrap();
    let protected = ProtectedPaths::new(vec![dir.path().join("secrets")]);
    let mut registry = ToolRegistry::new();
    register_all(&mut registry, protected);
    (registry, dir)
}

// ── read_file ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn read_file_success() {
    let (registry, dir) = make_registry();
    let path = dir.path().join("hello.txt");
    std::fs::write(&path, "hello world").unwrap();

    let result = registry
        .execute("read_file", json!({"path": path.to_str().unwrap()}))
        .await
        .unwrap();

    assert_eq!(result["contents"], "hello world");
}

#[tokio::test]
async fn read_file_not_found() {
    let (registry, dir) = make_registry();
    let result = registry
        .execute("read_file", json!({"path": dir.path().join("nope.txt").to_str().unwrap()}))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn read_file_protected_path() {
    let (registry, dir) = make_registry();
    let secrets_dir = dir.path().join("secrets");
    std::fs::create_dir_all(&secrets_dir).unwrap();
    std::fs::write(secrets_dir.join("api_key"), "supersecret").unwrap();

    let result = registry
        .execute(
            "read_file",
            json!({"path": secrets_dir.join("api_key").to_str().unwrap()}),
        )
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("not permitted") || err.contains("denied"), "got: {err}");
}

#[tokio::test]
async fn read_file_invalid_args() {
    let (registry, _dir) = make_registry();
    let result = registry.execute("read_file", json!({})).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("missing required field 'path'") || err.contains("invalid"), "got: {err}");
}

// ── write_file ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn write_file_success() {
    let (registry, dir) = make_registry();
    let path = dir.path().join("out.txt");

    let result = registry
        .execute(
            "write_file",
            json!({
                "path": path.to_str().unwrap(),
                "content": "wrote by test"
            }),
        )
        .await
        .unwrap();

    assert!(result["written"].as_u64().unwrap() > 0);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "wrote by test");
}

#[tokio::test]
async fn write_file_protected_path() {
    let (registry, dir) = make_registry();
    let secrets_dir = dir.path().join("secrets");
    std::fs::create_dir_all(&secrets_dir).unwrap();

    let result = registry
        .execute(
            "write_file",
            json!({
                "path": secrets_dir.join("evil.txt").to_str().unwrap(),
                "content": "bad"
            }),
        )
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn write_file_missing_content() {
    let (registry, dir) = make_registry();
    let path = dir.path().join("out.txt");
    let result = registry
        .execute("write_file", json!({"path": path.to_str().unwrap()}))
        .await;
    assert!(result.is_err());
}

// ── list_files ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn list_files_success() {
    let (registry, dir) = make_registry();
    std::fs::write(dir.path().join("a.txt"), "").unwrap();
    std::fs::write(dir.path().join("b.txt"), "").unwrap();
    std::fs::create_dir(dir.path().join("subdir")).unwrap();

    let result = registry
        .execute("list_files", json!({"path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();

    let entries = result["entries"].as_array().unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"a.txt"));
    assert!(names.contains(&"b.txt"));
    assert!(names.contains(&"subdir"));

    // subdir should be flagged as a directory.
    let subdir = entries.iter().find(|e| e["name"] == "subdir").unwrap();
    assert_eq!(subdir["is_dir"], true);
}

#[tokio::test]
async fn list_files_protected_path() {
    let (registry, dir) = make_registry();
    let secrets_dir = dir.path().join("secrets");
    std::fs::create_dir_all(&secrets_dir).unwrap();

    let result = registry
        .execute("list_files", json!({"path": secrets_dir.to_str().unwrap()}))
        .await;
    assert!(result.is_err());
}

// ── shell_exec ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn shell_exec_success() {
    let (registry, _dir) = make_registry();
    let result = registry
        .execute("shell_exec", json!({"command": "echo hello"}))
        .await
        .unwrap();

    assert_eq!(result["stdout"].as_str().unwrap().trim(), "hello");
    assert_eq!(result["exit_code"], 0);
}

#[tokio::test]
async fn shell_exec_nonzero_exit() {
    let (registry, _dir) = make_registry();
    let result = registry
        .execute("shell_exec", json!({"command": "exit 42"}))
        .await
        .unwrap();
    assert_eq!(result["exit_code"], 42);
}

#[tokio::test]
async fn shell_exec_protected_path_in_command() {
    let (registry, dir) = make_registry();
    let secrets_path = dir.path().join("secrets");
    let cmd = format!("cat {}", secrets_path.to_str().unwrap());

    let result = registry
        .execute("shell_exec", json!({"command": cmd}))
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("protected path") || err.contains("denied"), "got: {err}");
}

#[tokio::test]
async fn shell_exec_missing_command() {
    let (registry, _dir) = make_registry();
    let result = registry.execute("shell_exec", json!({})).await;
    assert!(result.is_err());
}

// ── registry meta ─────────────────────────────────────────────────────────────

#[test]
fn registry_schemas_sorted() {
    let protected = ProtectedPaths::new(vec![]);
    let mut registry = ToolRegistry::new();
    register_all(&mut registry, protected);

    let schemas = registry.schemas();
    let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "schemas should be sorted by name");
}

#[test]
fn registry_not_found() {
    let registry = ToolRegistry::new();
    // Verify the error variant
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(registry.execute("nonexistent", json!({})));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}
