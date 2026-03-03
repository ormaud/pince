pub mod agent;
pub mod frontend;
pub mod codec;

pub use agent::{AgentToSupervisor, SupervisorToAgent};
pub use frontend::{FrontendToSupervisor, SupervisorToFrontend};
pub use codec::Codec;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for an agent instance.
pub type AgentId = Uuid;
/// Unique identifier for a frontend connection.
pub type FrontendId = Uuid;
/// Unique identifier for a request/response correlation.
pub type RequestId = Uuid;

/// Common tool call representation shared between protocols.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: RequestId,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Outcome of a tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub id: RequestId,
    pub output: serde_json::Value,
}
