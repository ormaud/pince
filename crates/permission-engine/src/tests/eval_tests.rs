//! Unit tests for the evaluation logic.

use serde_json::json;

use crate::{Action, PolicyEngine};
use crate::policy::PolicyFile;

fn engine_from_toml(toml: &str) -> PolicyEngine {
    let policy = PolicyFile::parse(toml).expect("valid TOML");
    PolicyEngine::from_policy(policy)
}

#[tokio::test]
async fn default_deny_when_no_rules() {
    let engine = engine_from_toml("[defaults]\naction = \"deny\"");
    let result = engine.evaluate("agent1", "read_file", &json!({})).await;
    assert_eq!(result, Action::Deny);
}

#[tokio::test]
async fn default_allow_when_no_rules() {
    let engine = engine_from_toml("[defaults]\naction = \"allow\"");
    let result = engine.evaluate("agent1", "read_file", &json!({})).await;
    assert_eq!(result, Action::Allow);
}

#[tokio::test]
async fn explicit_allow_overrides_default_deny() {
    let toml = r#"
[defaults]
action = "deny"

[[rules]]
agent = "agent1"
tool = "read_file"
action = "allow"
"#;
    let engine = engine_from_toml(toml);
    assert_eq!(
        engine.evaluate("agent1", "read_file", &json!({})).await,
        Action::Allow
    );
    // Other agents still get denied.
    assert_eq!(
        engine.evaluate("agent2", "read_file", &json!({})).await,
        Action::Deny
    );
}

#[tokio::test]
async fn deny_takes_priority_over_allow() {
    let toml = r#"
[defaults]
action = "allow"

[[rules]]
agent = "*"
tool = "shell_exec"
action = "deny"

[[rules]]
agent = "*"
tool = "shell_exec"
action = "allow"
"#;
    let engine = engine_from_toml(toml);
    // deny rule appears first and wins.
    assert_eq!(
        engine.evaluate("any", "shell_exec", &json!({})).await,
        Action::Deny
    );
}

#[tokio::test]
async fn ask_rule_matched_when_no_deny_or_allow() {
    let toml = r#"
[defaults]
action = "deny"

[[rules]]
agent = "*"
tool = "write_file"
action = "ask"
"#;
    let engine = engine_from_toml(toml);
    assert_eq!(
        engine.evaluate("agent1", "write_file", &json!({})).await,
        Action::Ask
    );
}

#[tokio::test]
async fn wildcard_agent_matches_any() {
    let toml = r#"
[defaults]
action = "deny"

[[rules]]
agent = "*"
tool = "list_files"
action = "allow"
"#;
    let engine = engine_from_toml(toml);
    assert_eq!(
        engine.evaluate("alice", "list_files", &json!({})).await,
        Action::Allow
    );
    assert_eq!(
        engine.evaluate("bob", "list_files", &json!({})).await,
        Action::Allow
    );
}

#[tokio::test]
async fn path_glob_condition_allow() {
    let toml = r#"
[defaults]
action = "deny"

[[rules]]
agent = "*"
tool = "read_file"
action = "allow"
conditions = { path_glob = "/workspace/**" }
"#;
    let engine = engine_from_toml(toml);
    // Inside workspace → allow.
    assert_eq!(
        engine
            .evaluate("agent", "read_file", &json!({"path": "/workspace/foo.txt"}))
            .await,
        Action::Allow
    );
    // Outside workspace → default deny.
    assert_eq!(
        engine
            .evaluate("agent", "read_file", &json!({"path": "/etc/passwd"}))
            .await,
        Action::Deny
    );
}

#[tokio::test]
async fn command_regex_condition_deny() {
    let toml = r#"
[defaults]
action = "allow"

[[rules]]
agent = "*"
tool = "shell_exec"
conditions = { command_regex = "^rm\\s+-rf" }
action = "deny"
"#;
    let engine = engine_from_toml(toml);
    // Matches the destructive command → deny.
    assert_eq!(
        engine
            .evaluate("agent", "shell_exec", &json!({"command": "rm -rf /"}))
            .await,
        Action::Deny
    );
    // Safe command → allow (default).
    assert_eq!(
        engine
            .evaluate("agent", "shell_exec", &json!({"command": "ls -la"}))
            .await,
        Action::Allow
    );
}

#[tokio::test]
async fn condition_requires_matching_arg_key() {
    let toml = r#"
[defaults]
action = "deny"

[[rules]]
agent = "*"
tool = "read_file"
action = "allow"
conditions = { path_glob = "/workspace/**" }
"#;
    let engine = engine_from_toml(toml);
    // No `path` key at all → condition doesn't match → default deny.
    assert_eq!(
        engine
            .evaluate("agent", "read_file", &json!({"other": "value"}))
            .await,
        Action::Deny
    );
}

#[tokio::test]
async fn deny_before_allow_order() {
    // Rules are ordered: the deny-with-condition should win over allow-without-condition
    // because deny phase is checked first.
    let toml = r#"
[defaults]
action = "deny"

[[rules]]
agent = "*"
tool = "read_file"
action = "allow"

[[rules]]
agent = "*"
tool = "read_file"
conditions = { path_glob = "/secrets/**" }
action = "deny"
"#;
    let engine = engine_from_toml(toml);
    // /secrets/ path → deny wins over allow.
    assert_eq!(
        engine
            .evaluate("agent", "read_file", &json!({"path": "/secrets/key.txt"}))
            .await,
        Action::Deny
    );
    // Other path → allow wins.
    assert_eq!(
        engine
            .evaluate("agent", "read_file", &json!({"path": "/workspace/data.txt"}))
            .await,
        Action::Allow
    );
}
