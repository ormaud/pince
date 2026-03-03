//! Unit tests for hot reload.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tempfile::NamedTempFile;
use tokio::time::sleep;

use crate::{Action, PolicyEngine};
use crate::reload::watch_and_reload;

#[tokio::test]
async fn hot_reload_picks_up_policy_change() {
    // Start with deny-all.
    let mut global_file = NamedTempFile::new().unwrap();
    std::io::Write::write_all(
        global_file.as_file_mut(),
        b"[defaults]\naction = \"deny\"\n",
    )
    .unwrap();

    let engine = Arc::new(PolicyEngine::load(global_file.path(), None).unwrap());

    // Before change: denied.
    assert_eq!(
        engine.evaluate("agent", "read_file", &json!({})).await,
        Action::Deny
    );

    // Start watcher.
    let _guard = watch_and_reload(
        engine.clone(),
        global_file.path().to_path_buf(),
        None,
    )
    .unwrap();

    // Overwrite policy to allow.
    std::fs::write(
        global_file.path(),
        "[defaults]\naction = \"allow\"\n",
    )
    .unwrap();

    // Wait for debounce + reload.
    sleep(Duration::from_millis(600)).await;

    // After change: allowed.
    assert_eq!(
        engine.evaluate("agent", "read_file", &json!({})).await,
        Action::Allow
    );
}

#[tokio::test]
async fn reload_from_missing_global_uses_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let nonexistent = tmp.path().join("policy.toml");

    let engine = PolicyEngine::load(&nonexistent, None).unwrap();
    // Default policy (deny all).
    assert_eq!(
        engine.evaluate("agent", "read_file", &json!({})).await,
        Action::Deny
    );
}

#[tokio::test]
async fn manual_reload_via_engine() {
    let mut global_file = NamedTempFile::new().unwrap();
    std::io::Write::write_all(
        global_file.as_file_mut(),
        b"[defaults]\naction = \"deny\"\n",
    )
    .unwrap();

    let engine = PolicyEngine::load(global_file.path(), None).unwrap();

    assert_eq!(
        engine.evaluate("agent", "tool", &json!({})).await,
        Action::Deny
    );

    // Overwrite and manually reload.
    std::fs::write(global_file.path(), "[defaults]\naction = \"allow\"\n").unwrap();
    engine.reload(global_file.path(), None).await.unwrap();

    assert_eq!(
        engine.evaluate("agent", "tool", &json!({})).await,
        Action::Allow
    );
}
