//! Cron job configuration types.

use serde::{Deserialize, Serialize};

/// A single cron job definition loaded from the supervisor config.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CronJob {
    /// Unique name for this job (used in logs and audit).
    pub name: String,
    /// Cron expression (7-field: sec min hour dom month dow year).
    /// Example: "0 0 3 * * * *" = 3 AM every day.
    pub schedule: String,
    /// Agent profile name to spawn (e.g., "default").
    pub agent: String,
    /// Initial user message sent to the agent when it starts.
    pub prompt: String,
    /// Max runtime in seconds before the agent is killed. Default: 300.
    pub timeout_secs: Option<u64>,
    /// If false, skip this job entirely.
    pub enabled: bool,
}
