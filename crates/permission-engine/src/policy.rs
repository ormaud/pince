//! Policy file format: parsing and data structures.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The full contents of a policy TOML file.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct PolicyFile {
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub rules: Vec<PolicyRule>,
}

/// Global defaults section.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Defaults {
    pub action: Action,
}

impl Default for Defaults {
    fn default() -> Self {
        Self { action: Action::Deny }
    }
}

/// A single rule entry in the policy file.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PolicyRule {
    /// Agent name to match. Use `"*"` for any agent.
    pub agent: AgentMatcher,
    /// Tool name (exact string match).
    pub tool: String,
    /// Action to take when rule matches.
    pub action: Action,
    /// Optional argument conditions that must all be satisfied.
    #[serde(default)]
    pub conditions: Option<Conditions>,
}

/// Matches agent identifiers in rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentMatcher {
    /// Match any agent.
    Any,
    /// Match a specific agent by name.
    Named(String),
}

impl AgentMatcher {
    pub fn matches(&self, agent_id: &str) -> bool {
        match self {
            AgentMatcher::Any => true,
            AgentMatcher::Named(name) => name == agent_id,
        }
    }
}

// Custom deserialization: "*" maps to Any, anything else maps to Named.
impl<'de> serde::de::Deserialize<'de> for AgentMatcher {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s == "*" {
            Ok(AgentMatcher::Any)
        } else {
            Ok(AgentMatcher::Named(s))
        }
    }
}

impl serde::ser::Serialize for AgentMatcher {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        match self {
            AgentMatcher::Any => serializer.serialize_str("*"),
            AgentMatcher::Named(s) => serializer.serialize_str(s),
        }
    }
}

/// The outcome of a policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Deny,
    Ask,
}

/// Optional conditions that must hold for a rule to match.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Conditions {
    /// Glob pattern matched against path-like arguments (`path`, `file`, `filename`, `dir`).
    pub path_glob: Option<String>,
    /// Regex matched against the `command` argument (for shell_exec-like tools).
    pub command_regex: Option<String>,
}

impl Conditions {
    /// Returns true if all specified conditions match the given arguments.
    pub fn matches(&self, args: &serde_json::Value) -> bool {
        if let Some(glob_pat) = &self.path_glob {
            if !self.match_path_glob(glob_pat, args) {
                return false;
            }
        }
        if let Some(regex_pat) = &self.command_regex {
            if !self.match_command_regex(regex_pat, args) {
                return false;
            }
        }
        true
    }

    fn match_path_glob(&self, pattern: &str, args: &serde_json::Value) -> bool {
        // Extract path-like string from common argument keys.
        let path_keys = ["path", "file", "filename", "dir", "directory"];
        let path_val = path_keys.iter().find_map(|k| {
            args.get(k).and_then(|v| v.as_str())
        });

        match path_val {
            None => false, // no path argument → condition doesn't match
            Some(path) => {
                let pat = glob::Pattern::new(pattern).unwrap_or_else(|_| {
                    glob::Pattern::new("").unwrap()
                });
                pat.matches(path)
            }
        }
    }

    fn match_command_regex(&self, pattern: &str, args: &serde_json::Value) -> bool {
        let cmd_keys = ["command", "cmd", "shell", "exec"];
        let cmd_val = cmd_keys.iter().find_map(|k| {
            args.get(k).and_then(|v| v.as_str())
        });

        match cmd_val {
            None => false, // no command argument → condition doesn't match
            Some(cmd) => {
                regex::Regex::new(pattern)
                    .map(|re| re.is_match(cmd))
                    .unwrap_or(false)
            }
        }
    }
}

impl PolicyFile {
    /// Load and parse a policy TOML file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading policy file: {}", path.display()))?;
        Self::parse(&text)
    }

    /// Parse a policy from a TOML string.
    pub fn parse(text: &str) -> Result<Self> {
        let policy: Self = toml::from_str(text)
            .context("parsing policy TOML")?;
        Ok(policy)
    }

    /// Validate that this file's rules are acceptable as a project-local policy.
    ///
    /// Project-local policies may only add `deny` or `ask` rules — never `allow`.
    pub fn validate_project_local(&self) -> Result<()> {
        for rule in &self.rules {
            if rule.action == Action::Allow {
                bail!(
                    "project-local policy cannot add `allow` rules (tool={:?}, agent={:?})",
                    rule.tool, rule.agent
                );
            }
        }
        Ok(())
    }
}

/// Merge a global policy with an optional project-local policy.
///
/// Project-local rules are appended **before** global rules so that they take
/// higher precedence during evaluation (earlier rules in the list win).
pub fn merge_policies(global: PolicyFile, project: Option<PolicyFile>) -> PolicyFile {
    match project {
        None => global,
        Some(proj) => {
            // Project-local rules come first (highest priority).
            let mut merged_rules = proj.rules;
            merged_rules.extend(global.rules);
            PolicyFile {
                defaults: global.defaults,
                rules: merged_rules,
            }
        }
    }
}
