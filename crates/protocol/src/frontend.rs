//! Messages exchanged between external frontend clients and the supervisor.

use serde::{Deserialize, Serialize};

use crate::{AgentId, FrontendId, RequestId, ToolCall, ToolResult};

/// Messages the frontend sends to the supervisor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FrontendToSupervisor {
    /// Authenticate this frontend connection.
    Auth {
        token: String,
    },
    /// Send a chat message to the active agent.
    SendMessage {
        text: String,
    },
    /// Respond to a pending tool-call approval request.
    ApprovalResponse {
        request_id: RequestId,
        approved: bool,
    },
    /// List all known agents and their statuses.
    ListAgents,
}

/// Messages the supervisor sends to a frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SupervisorToFrontend {
    /// Result of an Auth message.
    AuthResult {
        frontend_id: FrontendId,
        success: bool,
        reason: Option<String>,
    },
    /// Incremental text from an agent response.
    AgentResponse {
        request_id: RequestId,
        agent_id: AgentId,
        text: String,
    },
    /// Agent has finished responding.
    AgentResponseDone {
        request_id: RequestId,
        agent_id: AgentId,
    },
    /// A tool call event (informational — tool already approved by policy).
    ToolCallEvent {
        agent_id: AgentId,
        tool_call: ToolCall,
    },
    /// Result of an executed tool call (informational).
    ToolResultEvent {
        agent_id: AgentId,
        tool_result: ToolResult,
    },
    /// The supervisor is requesting human approval for a tool call.
    ApprovalRequest {
        request_id: RequestId,
        agent_id: AgentId,
        tool_call: ToolCall,
    },
    /// Current list of agents.
    AgentList {
        agents: Vec<AgentInfo>,
    },
    /// An agent's status changed.
    AgentStatusChange {
        agent_id: AgentId,
        status: AgentStatus,
    },
    /// Supervisor-level error (connection will be closed).
    Error {
        message: String,
    },
}

/// Snapshot of an agent's state for `AgentList` / `AgentStatusChange`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub agent_id: AgentId,
    pub status: AgentStatus,
}

/// Possible lifecycle states for an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Starting,
    Ready,
    Processing,
    Dead,
}
