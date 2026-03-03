//! Unit tests for policy TOML parsing.

use crate::policy::{Action, AgentMatcher, PolicyFile};

#[test]
fn parses_empty_toml() {
    let policy = PolicyFile::parse("").expect("empty TOML should parse");
    assert_eq!(policy.defaults.action, Action::Deny);
    assert!(policy.rules.is_empty());
}

#[test]
fn parses_defaults_only() {
    let toml = r#"
[defaults]
action = "allow"
"#;
    let policy = PolicyFile::parse(toml).expect("should parse");
    assert_eq!(policy.defaults.action, Action::Allow);
    assert!(policy.rules.is_empty());
}

#[test]
fn parses_full_policy() {
    let toml = r#"
[defaults]
action = "deny"

[[rules]]
agent = "default"
tool = "read_file"
action = "allow"

[[rules]]
agent = "default"
tool = "write_file"
action = "ask"

[[rules]]
agent = "*"
tool = "list_files"
action = "allow"
conditions = { path_glob = "/workspace/**" }

[[rules]]
agent = "*"
tool = "shell_exec"
conditions = { command_regex = "^rm\\s+-rf" }
action = "deny"
"#;
    let policy = PolicyFile::parse(toml).expect("should parse");
    assert_eq!(policy.defaults.action, Action::Deny);
    assert_eq!(policy.rules.len(), 4);

    let r0 = &policy.rules[0];
    assert_eq!(r0.agent, AgentMatcher::Named("default".into()));
    assert_eq!(r0.tool, "read_file");
    assert_eq!(r0.action, Action::Allow);
    assert!(r0.conditions.is_none());

    let r2 = &policy.rules[2];
    assert_eq!(r2.agent, AgentMatcher::Any);
    assert!(r2.conditions.as_ref().unwrap().path_glob.is_some());

    let r3 = &policy.rules[3];
    assert!(r3.conditions.as_ref().unwrap().command_regex.is_some());
}

#[test]
fn rejects_invalid_toml() {
    let bad = "this is not toml = [";
    assert!(PolicyFile::parse(bad).is_err());
}

#[test]
fn rejects_unknown_action() {
    let bad = r#"
[defaults]
action = "maybe"
"#;
    assert!(PolicyFile::parse(bad).is_err());
}

#[test]
fn project_local_allows_deny_and_ask() {
    let toml = r#"
[[rules]]
agent = "*"
tool = "dangerous"
action = "deny"

[[rules]]
agent = "*"
tool = "risky"
action = "ask"
"#;
    let policy = PolicyFile::parse(toml).expect("should parse");
    assert!(policy.validate_project_local().is_ok());
}

#[test]
fn project_local_rejects_allow() {
    let toml = r#"
[[rules]]
agent = "*"
tool = "read_file"
action = "allow"
"#;
    let policy = PolicyFile::parse(toml).expect("should parse");
    assert!(policy.validate_project_local().is_err());
}
