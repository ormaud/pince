//! Tool schema definitions.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Risk classification for a tool, used by the permission engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// Read-only operations with no side effects.
    Safe,
    /// Operations that modify state but are recoverable.
    Sensitive,
    /// Operations with significant or irreversible side effects.
    Dangerous,
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RiskLevel::Safe => write!(f, "safe"),
            RiskLevel::Sensitive => write!(f, "sensitive"),
            RiskLevel::Dangerous => write!(f, "dangerous"),
        }
    }
}

/// The schema for a tool exposed to agents.
///
/// Agents receive only the schema (not the handler), and use it to
/// construct valid tool calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    /// Unique tool name (e.g. `read_file`, `shell_exec`).
    pub name: String,
    /// Human-readable description passed to LLMs for tool selection.
    pub description: String,
    /// JSON Schema (draft-07) describing the tool's input parameters.
    pub input_schema: Value,
    /// Risk classification for permission policy evaluation.
    pub risk_level: RiskLevel,
}

impl ToolSchema {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        risk_level: RiskLevel,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            risk_level,
        }
    }
}
