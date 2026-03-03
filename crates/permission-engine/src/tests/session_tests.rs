//! Unit tests for session-scoped approval overlay.

use serde_json::json;

use crate::{Action, PolicyEngine, SessionOverlay};
use crate::policy::PolicyFile;

#[test]
fn session_overlay_is_allowed() {
    let mut overlay = SessionOverlay::new();
    assert!(!overlay.is_allowed("agent1", "write_file"));

    overlay.add_allow(Some("agent1".into()), "write_file".into());
    assert!(overlay.is_allowed("agent1", "write_file"));
    // Different agent should NOT be allowed.
    assert!(!overlay.is_allowed("agent2", "write_file"));
}

#[test]
fn session_overlay_wildcard_agent() {
    let mut overlay = SessionOverlay::new();
    overlay.add_allow(None, "read_file".into()); // None = any agent
    assert!(overlay.is_allowed("agent1", "read_file"));
    assert!(overlay.is_allowed("agent2", "read_file"));
    assert!(!overlay.is_allowed("agent1", "write_file")); // Different tool.
}

#[test]
fn session_overlay_clear() {
    let mut overlay = SessionOverlay::new();
    overlay.add_allow(None, "write_file".into());
    overlay.clear();
    assert!(!overlay.is_allowed("agent1", "write_file"));
}

#[tokio::test]
async fn session_overlay_takes_priority_over_policy() {
    let toml = r#"
[defaults]
action = "deny"

[[rules]]
agent = "*"
tool = "write_file"
action = "deny"
"#;
    let policy = PolicyFile::parse(toml).unwrap();
    let engine = PolicyEngine::from_policy(policy);

    // Initially denied by policy.
    assert_eq!(
        engine.evaluate("agent1", "write_file", &json!({})).await,
        Action::Deny
    );

    // User approves for session.
    engine.add_session_allow(Some("agent1".into()), "write_file".into()).await;

    // Now allowed despite static policy.
    assert_eq!(
        engine.evaluate("agent1", "write_file", &json!({})).await,
        Action::Allow
    );

    // Other agent still denied.
    assert_eq!(
        engine.evaluate("agent2", "write_file", &json!({})).await,
        Action::Deny
    );
}
