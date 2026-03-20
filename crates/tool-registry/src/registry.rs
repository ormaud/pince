//! Central tool registry.

use std::collections::HashMap;

use serde_json::Value;

use crate::{ToolError, ToolHandler, ToolOutput, ToolSchema, validate};

/// A registered tool: its schema and its execution handler.
pub struct RegisteredTool {
    pub schema: ToolSchema,
    pub handler: Box<dyn ToolHandler>,
}

/// The central registry of all tools known to the supervisor.
///
/// Agents receive `Vec<ToolSchema>` at init time and never interact
/// with the registry directly — only the supervisor calls `execute`.
pub struct ToolRegistry {
    tools: HashMap<String, RegisteredTool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool. Replaces any existing tool with the same name.
    pub fn register(&mut self, schema: ToolSchema, handler: Box<dyn ToolHandler>) {
        let name = schema.name.clone();
        self.tools.insert(name, RegisteredTool { schema, handler });
    }

    /// Return the schemas for all registered tools (for sending to agents at init).
    pub fn schemas(&self) -> Vec<ToolSchema> {
        let mut schemas: Vec<_> = self.tools.values().map(|t| t.schema.clone()).collect();
        schemas.sort_by(|a, b| a.name.cmp(&b.name));
        schemas
    }

    /// Validate arguments against the tool's JSON Schema without executing.
    ///
    /// Returns `ToolError::NotFound` if the tool doesn't exist.
    /// Returns `ToolError::InvalidArguments` if the args fail schema validation.
    pub fn validate(&self, name: &str, args: &Value) -> Result<(), ToolError> {
        let tool = self.tools.get(name).ok_or_else(|| ToolError::NotFound(name.to_string()))?;
        validate::validate(&tool.schema.input_schema, args)
    }

    /// Validate arguments against the tool's JSON Schema, then execute the handler.
    ///
    /// Returns `ToolError::NotFound` if the tool doesn't exist.
    /// Returns `ToolError::InvalidArguments` if the args fail schema validation.
    pub async fn execute(&self, name: &str, args: Value) -> Result<ToolOutput, ToolError> {
        let tool = self.tools.get(name).ok_or_else(|| ToolError::NotFound(name.to_string()))?;

        validate::validate(&tool.schema.input_schema, &args)?;

        tool.handler.execute(args).await
    }

    /// Check whether a tool is registered.
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Return the number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
