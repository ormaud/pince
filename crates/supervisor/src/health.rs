//! `supervisor_health` built-in tool handler.
//!
//! Returns a JSON report of the supervisor's current status. This is handled
//! directly by the supervisor (not via the tool registry) because it needs
//! access to supervisor state.

use serde::{Deserialize, Serialize};

/// Status snapshot returned by the `supervisor_health` tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    /// Seconds since the supervisor process started.
    pub uptime_secs: u64,
    /// Number of active (non-dead) agents.
    pub active_agents: usize,
    /// Number of connected frontend clients.
    pub connected_frontends: usize,
    /// Total number of registered (enabled) cron jobs.
    pub scheduler_jobs: usize,
    /// True if the scheduler is running.
    pub scheduler_enabled: bool,
}

/// The name agents use to invoke this built-in tool.
pub const TOOL_NAME: &str = "supervisor_health";

/// JSON Schema for the `supervisor_health` input (no arguments).
pub fn input_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_report_serializes() {
        let report = HealthReport {
            uptime_secs: 42,
            active_agents: 2,
            connected_frontends: 1,
            scheduler_jobs: 3,
            scheduler_enabled: true,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["uptime_secs"], 42);
        assert_eq!(json["active_agents"], 2);
        assert_eq!(json["scheduler_enabled"], true);
    }

    #[test]
    fn input_schema_is_empty_object() {
        let schema = input_schema();
        assert_eq!(schema["type"], "object");
    }
}
