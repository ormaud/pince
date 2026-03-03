//! Tests for ToolRegistryConfig parsing.

use tool_registry::ToolRegistryConfig;

#[test]
fn parse_empty_config() {
    let config = ToolRegistryConfig::from_toml("").unwrap();
    assert!(config.mcp_servers.is_empty());
    assert!(config.protected_paths.is_empty());
}

#[test]
fn parse_mcp_server_config() {
    let toml = r#"
[[mcp_servers]]
name = "qmd"
command = "qmd"
args = ["mcp"]
"#;
    let config = ToolRegistryConfig::from_toml(toml).unwrap();
    assert_eq!(config.mcp_servers.len(), 1);
    let srv = &config.mcp_servers[0];
    assert_eq!(srv.name, "qmd");
    assert_eq!(srv.command, "qmd");
    assert_eq!(srv.args, vec!["mcp"]);
}

#[test]
fn parse_multiple_mcp_servers() {
    let toml = r#"
[[mcp_servers]]
name = "qmd"
command = "qmd"
args = ["mcp"]

[[mcp_servers]]
name = "other"
command = "other-tool"
args = []
"#;
    let config = ToolRegistryConfig::from_toml(toml).unwrap();
    assert_eq!(config.mcp_servers.len(), 2);
    assert_eq!(config.mcp_servers[0].name, "qmd");
    assert_eq!(config.mcp_servers[1].name, "other");
}

#[test]
fn parse_protected_paths() {
    let toml = r#"
protected_paths = ["/run/pince/secrets", "/tmp/keys"]
"#;
    let config = ToolRegistryConfig::from_toml(toml).unwrap();
    assert_eq!(config.protected_paths.len(), 2);
}
