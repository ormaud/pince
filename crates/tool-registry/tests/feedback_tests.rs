//! Tests for the `feedback` built-in tool.

use std::sync::Arc;

use mockito::Server;
use serde_json::json;
use tempfile::TempDir;
use tool_registry::{FeedbackConfig, ToolRegistry, builtin::register_feedback};
use secrets::SecretStore;

fn make_feedback_registry(api_base_url: &str) -> (ToolRegistry, TempDir) {
    let dir = TempDir::new().unwrap();
    let store = SecretStore::new(dir.path().join("secrets")).unwrap();
    store.set("trame-api-key", b"test-api-key-abc").unwrap();

    let config = FeedbackConfig {
        api_base_url: api_base_url.to_string(),
        secret_store: Arc::new(store),
    };
    let mut registry = ToolRegistry::new();
    register_feedback(&mut registry, config);
    (registry, dir)
}

#[tokio::test]
async fn feedback_missing_content() {
    let mut server = Server::new_async().await;
    let (registry, _dir) = make_feedback_registry(&server.url());

    // Ensure no HTTP call is made — a mock that panics if called
    let _m = server
        .mock("POST", "/api/feedback")
        .expect(0)
        .create_async()
        .await;

    let result = registry
        .execute("feedback", json!({"project_id": "some-uuid"}))
        .await;

    assert!(result.is_err());
    assert!(
        result.unwrap_err().to_string().contains("content"),
        "expected missing content error"
    );
}

#[tokio::test]
async fn feedback_missing_project_id() {
    let mut server = Server::new_async().await;
    let (registry, _dir) = make_feedback_registry(&server.url());

    let _m = server
        .mock("POST", "/api/feedback")
        .expect(0)
        .create_async()
        .await;

    let result = registry
        .execute("feedback", json!({"content": "great tool!"}))
        .await;

    assert!(result.is_err());
    assert!(
        result.unwrap_err().to_string().contains("project_id"),
        "expected missing project_id error"
    );
}

#[tokio::test]
async fn feedback_success() {
    let mut server = Server::new_async().await;
    let (registry, _dir) = make_feedback_registry(&server.url());

    let mock = server
        .mock("POST", "/api/feedback")
        .match_header("authorization", "Bearer test-api-key-abc")
        .match_header("content-type", mockito::Matcher::Regex("application/json".into()))
        .with_status(201)
        .with_body(r#"{"id":"fb-123"}"#)
        .create_async()
        .await;

    let result = registry
        .execute(
            "feedback",
            json!({
                "content": "This is great feedback!",
                "project_id": "ac103d9c-4e2f-4b4d-a902-15ca4b9ad610"
            }),
        )
        .await
        .unwrap();

    assert_eq!(result["status"], "submitted");
    mock.assert_async().await;
}

#[tokio::test]
async fn feedback_api_error() {
    let mut server = Server::new_async().await;
    let (registry, _dir) = make_feedback_registry(&server.url());

    let mock = server
        .mock("POST", "/api/feedback")
        .with_status(403)
        .with_body("Forbidden")
        .create_async()
        .await;

    let result = registry
        .execute(
            "feedback",
            json!({
                "content": "some feedback",
                "project_id": "ac103d9c-4e2f-4b4d-a902-15ca4b9ad610"
            }),
        )
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("403"), "expected 403 in error, got: {err}");
    mock.assert_async().await;
}

#[tokio::test]
async fn feedback_missing_api_key() {
    let mut server = Server::new_async().await;

    // Create a store WITHOUT the trame-api-key secret
    let dir = TempDir::new().unwrap();
    let store = SecretStore::new(dir.path().join("secrets")).unwrap();

    let config = FeedbackConfig {
        api_base_url: server.url(),
        secret_store: Arc::new(store),
    };
    let mut registry = ToolRegistry::new();
    register_feedback(&mut registry, config);

    let _m = server
        .mock("POST", "/api/feedback")
        .expect(0)
        .create_async()
        .await;

    let result = registry
        .execute(
            "feedback",
            json!({
                "content": "feedback text",
                "project_id": "ac103d9c-4e2f-4b4d-a902-15ca4b9ad610"
            }),
        )
        .await;

    assert!(result.is_err());
    assert!(
        result.unwrap_err().to_string().contains("API key not found"),
        "expected API key not found error"
    );
}

#[test]
fn feedback_schema_is_sensitive() {
    use tool_registry::builtin::feedback::schema;
    use tool_registry::RiskLevel;
    let s = schema();
    assert_eq!(s.name, "feedback");
    assert_eq!(s.risk_level, RiskLevel::Sensitive);
    assert!(s.input_schema["required"].as_array().unwrap().contains(&serde_json::json!("content")));
    assert!(s.input_schema["required"].as_array().unwrap().contains(&serde_json::json!("project_id")));
}
