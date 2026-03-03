"""Tests for the agent main loop logic."""
from __future__ import annotations

import asyncio
import json
import struct
import sys
import os
from unittest.mock import AsyncMock, MagicMock, patch
import pytest

from pince_agent.agent import agent_loop, process_llm_turn, HEARTBEAT_INTERVAL
from pince_agent.protocol import write_message, read_message

from pince_proto import (
    AgentConfig,
    AgentMessage,
    Init,
    Ready,
    ResponseChunk,
    ResponseDone,
    Shutdown,
    SupervisorMessage,
    ToolCallDenied,
    ToolCallRequest,
    ToolCallResult,
    ToolSchema,
    UserMessage,
)


def make_stream_pair():
    """In-memory asyncio stream pair."""
    queue = asyncio.Queue()

    class FakeReader:
        def __init__(self, q):
            self._q = q
            self._buf = b""

        async def readexactly(self, n):
            while len(self._buf) < n:
                chunk = await self._q.get()
                self._buf += chunk
            data, self._buf = self._buf[:n], self._buf[n:]
            return data

    class FakeWriter:
        def __init__(self, q):
            self._q = q
            self.closed = False

        def write(self, data):
            self._q.put_nowait(data)

        async def drain(self):
            pass

    return FakeReader(queue), FakeWriter(queue)


def make_config(model="claude-opus-4-6", system="You are a helpful assistant.", tools=None):
    config = AgentConfig()
    config.agent_id = "test-agent-1"
    config.model = model
    config.system_prompt = system
    config.max_tokens = 1024
    if tools:
        for t in tools:
            config.tools.append(t)
    return config


def make_supervisor_shutdown() -> bytes:
    """Serialize a SupervisorMessage(shutdown={}) to wire format."""
    msg = SupervisorMessage()
    msg.shutdown.CopyFrom(Shutdown())
    data = msg.SerializeToString()
    return struct.pack(">I", len(data)) + data


def make_supervisor_user_message(content: str) -> bytes:
    msg = SupervisorMessage()
    msg.user_message.content = content
    data = msg.SerializeToString()
    return struct.pack(">I", len(data)) + data


def make_supervisor_tool_result(request_id: str, result: str) -> bytes:
    msg = SupervisorMessage()
    msg.tool_result.request_id = request_id
    msg.tool_result.result_json = result.encode()
    data = msg.SerializeToString()
    return struct.pack(">I", len(data)) + data


def make_supervisor_tool_denied(request_id: str, reason: str) -> bytes:
    msg = SupervisorMessage()
    msg.tool_denied.request_id = request_id
    msg.tool_denied.reason = reason
    data = msg.SerializeToString()
    return struct.pack(">I", len(data)) + data


# ---------------------------------------------------------------------------
# process_llm_turn tests (mocked Anthropic client)
# ---------------------------------------------------------------------------

class MockStreamEvent:
    """Minimal mock for Anthropic stream events."""
    pass


def make_text_delta_event(text: str):
    import anthropic
    delta = MagicMock(spec=anthropic.types.TextDelta)
    delta.text = text
    event = MagicMock(spec=anthropic.types.RawContentBlockDeltaEvent)
    event.delta = delta
    return event


def make_stop_event(stop_reason: str = "end_turn"):
    import anthropic
    delta = MagicMock()
    delta.stop_reason = stop_reason
    event = MagicMock(spec=anthropic.types.RawMessageDeltaEvent)
    event.delta = delta
    return event


def make_tool_use_start_event(tool_id: str, tool_name: str):
    import anthropic
    block = MagicMock(spec=anthropic.types.ToolUseBlock)
    block.id = tool_id
    block.name = tool_name
    event = MagicMock(spec=anthropic.types.RawContentBlockStartEvent)
    event.content_block = block
    return event


def make_tool_input_delta_event(partial_json: str):
    import anthropic
    delta = MagicMock(spec=anthropic.types.InputJSONDelta)
    delta.partial_json = partial_json
    event = MagicMock(spec=anthropic.types.RawContentBlockDeltaEvent)
    event.delta = delta
    return event


async def fake_stream(events):
    for event in events:
        yield event


@pytest.mark.asyncio
async def test_process_llm_turn_text_response():
    """process_llm_turn sends ResponseChunk events and ResponseDone."""
    import anthropic

    supervisor_reader, agent_writer = make_stream_pair()
    conversation = [{"role": "user", "content": "Hello"}]

    events = [
        make_text_delta_event("Hi "),
        make_text_delta_event("there!"),
        make_stop_event("end_turn"),
    ]

    mock_client = MagicMock()
    mock_client.stream_response = MagicMock(return_value=fake_stream(events))

    result = await process_llm_turn(mock_client, conversation, [], agent_writer)
    assert result is None  # no tool call

    # Read back messages from writer queue
    messages = []
    while not agent_writer._q.empty():
        # Read framed message
        buf = b""
        while True:
            chunk = agent_writer._q.get_nowait()
            buf += chunk
            if len(buf) >= 4:
                break

    # Drain all written data
    all_data = b""
    while not agent_writer._q.empty():
        all_data += agent_writer._q.get_nowait()

    # Check conversation was updated
    assert len(conversation) == 2  # user + assistant
    assert conversation[1]["role"] == "assistant"
    assert conversation[1]["content"][0]["text"] == "Hi there!"


@pytest.mark.asyncio
async def test_process_llm_turn_tool_call():
    """process_llm_turn sends ToolCallRequest when LLM uses a tool."""
    import anthropic

    _, agent_writer = make_stream_pair()
    conversation = [{"role": "user", "content": "What time is it?"}]

    events = [
        make_tool_use_start_event("tool-123", "get_time"),
        make_tool_input_delta_event('{"tz": "UTC"}'),
        make_stop_event("tool_use"),
    ]

    mock_client = MagicMock()
    mock_client.stream_response = MagicMock(return_value=fake_stream(events))

    result = await process_llm_turn(mock_client, conversation, [], agent_writer)
    assert result == "tool-123"

    # conversation should have assistant turn with tool_use
    assert len(conversation) == 2
    assert conversation[1]["role"] == "assistant"
    tool_use_block = conversation[1]["content"][0]
    assert tool_use_block["type"] == "tool_use"
    assert tool_use_block["name"] == "get_time"
    assert tool_use_block["input"] == {"tz": "UTC"}


@pytest.mark.asyncio
async def test_agent_loop_shutdown():
    """agent_loop exits cleanly on Shutdown message."""
    import anthropic

    # Supervisor side: write a Shutdown message
    sup_reader, agent_writer = make_stream_pair()
    agent_reader, sup_writer = make_stream_pair()

    # Write shutdown to agent_reader queue
    shutdown_bytes = make_supervisor_shutdown()
    for byte in [shutdown_bytes]:
        agent_reader._q.put_nowait(byte)

    config = make_config()
    mock_client = MagicMock()
    mock_client.stream_response = MagicMock(return_value=fake_stream([]))

    with patch("pince_agent.agent.AnthropicClient", return_value=mock_client):
        # Use a short timeout to prevent test hanging
        await asyncio.wait_for(
            agent_loop(agent_reader, agent_writer, config),
            timeout=2.0,
        )


@pytest.mark.asyncio
async def test_agent_loop_user_message_then_shutdown():
    """agent_loop processes a user message and then shuts down."""
    import anthropic

    agent_reader, sup_writer = make_stream_pair()

    # Write: user_message, then shutdown
    user_msg_bytes = make_supervisor_user_message("Hello agent!")
    shutdown_bytes = make_supervisor_shutdown()
    agent_reader._q.put_nowait(user_msg_bytes)
    agent_reader._q.put_nowait(shutdown_bytes)

    _, agent_writer = make_stream_pair()

    events = [make_text_delta_event("Hi!"), make_stop_event("end_turn")]
    mock_client = MagicMock()
    mock_client.stream_response = MagicMock(return_value=fake_stream(events))

    config = make_config()
    with patch("pince_agent.agent.AnthropicClient", return_value=mock_client):
        await asyncio.wait_for(
            agent_loop(agent_reader, agent_writer, config),
            timeout=3.0,
        )

    # Verify the mock was called once (for the user message)
    mock_client.stream_response.assert_called_once()


@pytest.mark.asyncio
async def test_agent_loop_tool_denied():
    """agent_loop handles tool_denied by continuing LLM turn."""
    import anthropic

    agent_reader, _ = make_stream_pair()
    _, agent_writer = make_stream_pair()

    # User message → tool call → denied → LLM responds → shutdown
    user_msg_bytes = make_supervisor_user_message("List files")
    denied_bytes = make_supervisor_tool_denied("tool-abc", "permission denied")
    shutdown_bytes = make_supervisor_shutdown()

    agent_reader._q.put_nowait(user_msg_bytes)
    agent_reader._q.put_nowait(denied_bytes)
    agent_reader._q.put_nowait(shutdown_bytes)

    call_count = 0

    async def fake_stream_fn(messages, tools):
        nonlocal call_count
        call_count += 1
        if call_count == 1:
            # First call: request tool use
            yield make_tool_use_start_event("tool-abc", "list_files")
            yield make_tool_input_delta_event("{}")
            yield make_stop_event("tool_use")
        else:
            # Second call: normal response after denial
            yield make_text_delta_event("I cannot list files.")
            yield make_stop_event("end_turn")

    mock_client = MagicMock()
    mock_client.stream_response = MagicMock(side_effect=fake_stream_fn)

    config = make_config()
    with patch("pince_agent.agent.AnthropicClient", return_value=mock_client):
        await asyncio.wait_for(
            agent_loop(agent_reader, agent_writer, config),
            timeout=3.0,
        )

    assert call_count == 2


# ---------------------------------------------------------------------------
# anthropic_client tests
# ---------------------------------------------------------------------------

def test_convert_tool_schemas_empty():
    from pince_agent.anthropic_client import convert_tool_schemas
    assert convert_tool_schemas([]) == []


def test_convert_tool_schemas_basic():
    from pince_agent.anthropic_client import convert_tool_schemas

    schema = MagicMock()
    schema.name = "my_tool"
    schema.description = "Does a thing"
    schema.input_schema_json = b'{"type": "object", "properties": {}}'

    result = convert_tool_schemas([schema])
    assert len(result) == 1
    assert result[0]["name"] == "my_tool"
    assert result[0]["description"] == "Does a thing"
    assert result[0]["input_schema"] == {"type": "object", "properties": {}}
