"""Main agent loop for the pince sub-agent."""
from __future__ import annotations

import asyncio
import json
import logging
import sys
from typing import Any

import anthropic

from pince_agent.anthropic_client import AnthropicClient, convert_tool_schemas
from pince_agent.protocol import read_message, write_message

# Import generated protobuf types (from sibling pince_proto package)
import pathlib as _pathlib
_agent_dir = str(_pathlib.Path(__file__).parent.parent)
sys.path.insert(0, _agent_dir)
# frontend_pb2 uses bare `import agent_pb2`, so pince_proto dir must be on path
sys.path.insert(0, str(_pathlib.Path(__file__).parent.parent / "pince_proto"))
from pince_proto import (  # noqa: E402
    AgentMessage,
    AgentError,
    Heartbeat,
    Ready,
    ResponseChunk,
    ResponseDone,
    SupervisorMessage,
    ToolCallRequest,
)

logger = logging.getLogger(__name__)

HEARTBEAT_INTERVAL = 15.0  # seconds


async def heartbeat_loop(writer: asyncio.StreamWriter) -> None:
    """Send a Heartbeat to the supervisor every HEARTBEAT_INTERVAL seconds."""
    while True:
        await asyncio.sleep(HEARTBEAT_INTERVAL)
        try:
            msg = AgentMessage()
            msg.heartbeat.CopyFrom(Heartbeat())
            await write_message(writer, msg)
            logger.debug("heartbeat sent")
        except Exception:
            logger.warning("heartbeat write failed — exiting")
            return


async def send_error(writer: asyncio.StreamWriter, message: str) -> None:
    msg = AgentMessage()
    msg.error.message = message
    await write_message(writer, msg)


async def process_llm_turn(
    client: AnthropicClient,
    conversation: list[dict[str, Any]],
    tools: list[dict[str, Any]],
    writer: asyncio.StreamWriter,
) -> str | None:
    """Call the LLM, stream chunks to supervisor.

    Returns the tool call request_id if a tool was requested, else None.
    """
    current_text = ""
    tool_use_id: str | None = None
    tool_use_name: str | None = None
    tool_use_input_json = ""
    stop_reason: str | None = None
    assistant_content: list[dict[str, Any]] = []

    try:
        async for event in client.stream_response(conversation, tools):
            if isinstance(event, anthropic.types.RawContentBlockDeltaEvent):
                delta = event.delta
                if isinstance(delta, anthropic.types.TextDelta):
                    current_text += delta.text
                    chunk_msg = AgentMessage()
                    chunk_msg.response.content = delta.text
                    await write_message(writer, chunk_msg)
                elif isinstance(delta, anthropic.types.InputJSONDelta):
                    tool_use_input_json += delta.partial_json

            elif isinstance(event, anthropic.types.RawContentBlockStartEvent):
                block = event.content_block
                if isinstance(block, anthropic.types.ToolUseBlock):
                    tool_use_id = block.id
                    tool_use_name = block.name
                    tool_use_input_json = ""

            elif isinstance(event, anthropic.types.RawMessageDeltaEvent):
                stop_reason = event.delta.stop_reason

    except anthropic.APIError as e:
        logger.error("Anthropic API error: %s", e)
        await send_error(writer, f"LLM API error: {e}")
        return None

    # Build assistant message content for conversation history
    if current_text:
        assistant_content.append({"type": "text", "text": current_text})

    if tool_use_id and tool_use_name:
        try:
            parsed_input = json.loads(tool_use_input_json) if tool_use_input_json else {}
        except json.JSONDecodeError:
            parsed_input = {}

        assistant_content.append(
            {
                "type": "tool_use",
                "id": tool_use_id,
                "name": tool_use_name,
                "input": parsed_input,
            }
        )

    # Append assistant turn to conversation
    if assistant_content:
        conversation.append({"role": "assistant", "content": assistant_content})

    if stop_reason == "end_turn" or (stop_reason != "tool_use" and not tool_use_id):
        # Send ResponseDone
        done_msg = AgentMessage()
        done_msg.response_done.CopyFrom(ResponseDone())
        await write_message(writer, done_msg)
        return None

    if tool_use_id and tool_use_name:
        # Send ToolCallRequest to supervisor
        tool_msg = AgentMessage()
        tool_msg.tool_call.request_id = tool_use_id
        tool_msg.tool_call.tool = tool_use_name
        tool_msg.tool_call.arguments_json = (
            tool_use_input_json.encode() if tool_use_input_json else b"{}"
        )
        await write_message(writer, tool_msg)
        return tool_use_id

    # Fallback: send done
    done_msg = AgentMessage()
    done_msg.response_done.CopyFrom(ResponseDone())
    await write_message(writer, done_msg)
    return None


async def agent_loop(
    reader: asyncio.StreamReader,
    writer: asyncio.StreamWriter,
    config: Any,
) -> None:
    """Main agent event loop."""
    client = AnthropicClient(
        model=config.model,
        system_prompt=config.system_prompt,
        max_tokens=config.max_tokens or 4096,
    )
    tools = convert_tool_schemas(config.tools)
    conversation: list[dict[str, Any]] = []

    # Start heartbeat task
    heartbeat_task = asyncio.create_task(heartbeat_loop(writer))

    try:
        while True:
            msg = await read_message(reader, SupervisorMessage)

            if msg.HasField("user_message"):
                conversation.append({"role": "user", "content": msg.user_message.content})
                await process_llm_turn(client, conversation, tools, writer)

            elif msg.HasField("tool_result"):
                conversation.append(
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "tool_result",
                                "tool_use_id": msg.tool_result.request_id,
                                "content": json.loads(msg.tool_result.result_json),
                            }
                        ],
                    }
                )
                await process_llm_turn(client, conversation, tools, writer)

            elif msg.HasField("tool_denied"):
                conversation.append(
                    {
                        "role": "user",
                        "content": [
                            {
                                "type": "tool_result",
                                "tool_use_id": msg.tool_denied.request_id,
                                "content": f"Tool call denied: {msg.tool_denied.reason}",
                                "is_error": True,
                            }
                        ],
                    }
                )
                await process_llm_turn(client, conversation, tools, writer)

            elif msg.HasField("cancel"):
                logger.info("received cancel — ignoring (no active streaming)")

            elif msg.HasField("shutdown"):
                logger.info("received shutdown")
                break

    finally:
        heartbeat_task.cancel()
        try:
            await heartbeat_task
        except asyncio.CancelledError:
            pass
