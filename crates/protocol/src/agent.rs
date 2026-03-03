//! Messages exchanged between the supervisor and a sub-agent process.
//!
//! Direction conventions:
//! - `AgentToSupervisor` – messages sent FROM the agent TO the supervisor
//! - `SupervisorToAgent` – messages sent FROM the supervisor TO the agent

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AgentId, RequestId, ToolCall, ToolResult};

/// Messages the agent sends to the supervisor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentToSupervisor {
    /// Agent is ready to receive messages (sent after startup).
    Ready {
        agent_id: AgentId,
    },
    /// Agent is requesting a tool call. Supervisor will evaluate permissions.
    ToolCall(ToolCall),
    /// Incremental response text chunk.
    Response {
        request_id: RequestId,
        text: String,
    },
    /// Agent has finished producing a response.
    ResponseDone {
        request_id: RequestId,
    },
    /// Liveness signal — supervisor kills agent if heartbeats stop.
    Heartbeat,
    /// Agent encountered an unrecoverable error.
    Error {
        message: String,
    },
}

/// Messages the supervisor sends to the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SupervisorToAgent {
    /// Assign the agent its identity (sent immediately after connection).
    Init {
        agent_id: AgentId,
    },
    /// Forward a user message for the agent to process.
    UserMessage {
        request_id: RequestId,
        text: String,
    },
    /// Result of an approved tool call.
    ToolResult(ToolResult),
    /// The tool call was denied by the permission engine or user.
    ToolDenied {
        id: RequestId,
        reason: String,
    },
    /// Request the agent to cancel a pending request.
    Cancel {
        request_id: RequestId,
    },
    /// Request the agent to shut down gracefully.
    Shutdown,
}

/// A unique identifier for an agent session, wrapped for clarity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentSessionId(pub Uuid);

impl AgentSessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AgentSessionId {
    fn default() -> Self {
        Self::new()
    }
}
