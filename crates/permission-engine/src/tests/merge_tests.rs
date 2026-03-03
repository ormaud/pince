//! Unit tests for policy merging (global + project-local).

use serde_json::json;

use crate::{Action, PolicyEngine};
use crate::policy::{merge_policies, PolicyFile};

#[test]
fn merge_project_rules_take_precedence() {
    let global = PolicyFile::parse(r#"
[defaults]
action = "allow"

[[rules]]
agent = "*"
tool = "dangerous"
action = "ask"
"#).unwrap();

    let project = PolicyFile::parse(r#"
[[rules]]
agent = "*"
tool = "dangerous"
action = "deny"
"#).unwrap();

    let merged = merge_policies(global, Some(project));
    // Project-local deny rule should appear first and take priority.
    assert_eq!(merged.rules[0].action, Action::Deny);
    assert_eq!(merged.rules[1].action, Action::Ask);
}

#[test]
fn merge_without_project_uses_global_only() {
    let global = PolicyFile::parse(r#"
[defaults]
action = "deny"

[[rules]]
agent = "*"
tool = "read_file"
action = "allow"
"#).unwrap();

    let merged = merge_policies(global, None);
    assert_eq!(merged.rules.len(), 1);
    assert_eq!(merged.rules[0].action, Action::Allow);
}

#[tokio::test]
async fn project_local_cannot_widen_to_allow() {
    let bad_project = PolicyFile::parse(r#"
[[rules]]
agent = "*"
tool = "shell_exec"
action = "allow"
"#).unwrap();
    assert!(bad_project.validate_project_local().is_err());
}

#[tokio::test]
async fn merged_engine_respects_project_deny() {
    let global = PolicyFile::parse(r#"
[defaults]
action = "deny"

[[rules]]
agent = "*"
tool = "list_files"
action = "allow"
"#).unwrap();

    let project = PolicyFile::parse(r#"
[[rules]]
agent = "*"
tool = "list_files"
conditions = { path_glob = "/secrets/**" }
action = "deny"
"#).unwrap();

    let merged = merge_policies(global, Some(project));
    let engine = PolicyEngine::from_policy(merged);

    // Normal path → global allow.
    assert_eq!(
        engine.evaluate("agent", "list_files", &json!({"path": "/workspace/files"})).await,
        Action::Allow
    );
    // Secrets path → project deny overrides global allow.
    assert_eq!(
        engine.evaluate("agent", "list_files", &json!({"path": "/secrets/keys"})).await,
        Action::Deny
    );
}
