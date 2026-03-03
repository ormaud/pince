# pince_proto - Generated protobuf types for the pince agent-supervisor protocol.
#
# Generated from proto/agent.proto using protoc.
# Do not edit by hand — regenerate with: scripts/gen_proto.sh
from . import agent_pb2
from .agent_pb2 import (
    AgentMessage,
    SupervisorMessage,
    Ready,
    ToolCallRequest,
    ResponseChunk,
    ResponseDone,
    AgentError,
    Heartbeat,
    Init,
    AgentConfig,
    ToolSchema,
    UserMessage,
    ToolCallResult,
    ToolCallDenied,
    Cancel,
    Shutdown,
    RiskLevel,
)

__all__ = [
    "agent_pb2",
    "AgentMessage",
    "SupervisorMessage",
    "Ready",
    "ToolCallRequest",
    "ResponseChunk",
    "ResponseDone",
    "AgentError",
    "Heartbeat",
    "Init",
    "AgentConfig",
    "ToolSchema",
    "UserMessage",
    "ToolCallResult",
    "ToolCallDenied",
    "Cancel",
    "Shutdown",
    "RiskLevel",
]
