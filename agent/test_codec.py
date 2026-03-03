"""Tests for the Python framing layer and auth token protocol."""
import asyncio
import struct
import sys
import os

# Ensure the agent directory and pince_proto are importable
sys.path.insert(0, os.path.dirname(__file__))

import pytest
from codec import (
    read_message,
    write_message,
    read_raw_frame,
    write_raw_frame,
    MessageTooLargeError,
    MAX_MESSAGE_SIZE,
)
from auth import (
    recv_auth_token,
    send_auth_token,
    parse_token,
    AuthError,
    AUTH_TOKEN_LEN,
)
from pince_proto import AgentMessage, SupervisorMessage, Ready, ResponseChunk, Heartbeat


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def make_stream_pair():
    """Create an in-memory stream pair for testing."""
    r1, w1 = asyncio.Queue(), asyncio.Queue()

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

        def write(self, data):
            self._q.put_nowait(data)

        async def drain(self):
            pass

    return FakeReader(r1), FakeWriter(r1)


# ---------------------------------------------------------------------------
# Codec round-trip tests
# ---------------------------------------------------------------------------

@pytest.mark.asyncio
async def test_round_trip_ready():
    reader, writer = make_stream_pair()
    msg = AgentMessage()
    msg.ready.CopyFrom(Ready())

    await write_message(writer, msg)
    decoded = await read_message(reader, AgentMessage)

    assert decoded.HasField("ready")


@pytest.mark.asyncio
async def test_round_trip_response_chunk():
    reader, writer = make_stream_pair()
    msg = AgentMessage()
    msg.response.content = "Hello, world!"

    await write_message(writer, msg)
    decoded = await read_message(reader, AgentMessage)

    assert decoded.HasField("response")
    assert decoded.response.content == "Hello, world!"


@pytest.mark.asyncio
async def test_round_trip_heartbeat():
    reader, writer = make_stream_pair()
    msg = AgentMessage()
    msg.heartbeat.CopyFrom(Heartbeat())

    await write_message(writer, msg)
    decoded = await read_message(reader, AgentMessage)

    assert decoded.HasField("heartbeat")


@pytest.mark.asyncio
async def test_raw_frame_round_trip():
    reader, writer = make_stream_pair()
    token = b"my-secret-auth-token-32bytes-xxxx"

    await write_raw_frame(writer, token)
    decoded = await read_raw_frame(reader)

    assert decoded == token


@pytest.mark.asyncio
async def test_oversized_message_rejected_on_read():
    reader, writer = make_stream_pair()
    # Write a header claiming to be oversized
    fake_len = MAX_MESSAGE_SIZE + 1
    writer.write(struct.pack(">I", fake_len))

    with pytest.raises(MessageTooLargeError):
        await read_message(reader, AgentMessage)


@pytest.mark.asyncio
async def test_empty_message_round_trip():
    """An empty protobuf message serializes to 0 bytes."""
    reader, writer = make_stream_pair()
    msg = AgentMessage()
    msg.ready.CopyFrom(Ready())

    await write_message(writer, msg)
    decoded = await read_message(reader, AgentMessage)
    assert decoded.HasField("ready")


# ---------------------------------------------------------------------------
# Cross-language compatibility: wire format checks
# ---------------------------------------------------------------------------

def test_wire_format_known_bytes():
    """The Python serialization of AgentMessage{ready: {}} must match known bytes.

    prost and protobuf agree on the encoding:
    - field 1 (ready), wire type 2 (length-delimited) -> tag 0x0a
    - length 0 -> 0x00
    So: b'\\x0a\\x00'
    """
    msg = AgentMessage()
    msg.ready.CopyFrom(Ready())
    data = msg.SerializeToString()
    assert data == b"\x0a\x00", f"unexpected encoding: {data.hex()}"


def test_wire_format_response_chunk():
    """ResponseChunk{content: 'hi'} -> field 3, wire type 2, length 2, 'hi'."""
    msg = AgentMessage()
    msg.response.content = "hi"
    data = msg.SerializeToString()
    # field 3 (response) -> tag byte = (3 << 3) | 2 = 0x1a
    assert data[0:1] == b"\x1a"


# ---------------------------------------------------------------------------
# Auth token tests
# ---------------------------------------------------------------------------

def test_parse_token_valid():
    hex_token = "aa" * AUTH_TOKEN_LEN
    token = parse_token(hex_token)
    assert len(token) == AUTH_TOKEN_LEN
    assert token == bytes([0xAA] * AUTH_TOKEN_LEN)


def test_parse_token_wrong_length():
    with pytest.raises(AuthError):
        parse_token("aabb")
    with pytest.raises(AuthError):
        parse_token("")


def test_parse_token_invalid_hex():
    with pytest.raises(AuthError):
        parse_token("zz" * AUTH_TOKEN_LEN)


@pytest.mark.asyncio
async def test_auth_round_trip():
    reader, writer = make_stream_pair()
    token = bytes([0xDE] * AUTH_TOKEN_LEN)

    await send_auth_token(writer, token)
    await recv_auth_token(reader, token)  # Should not raise


@pytest.mark.asyncio
async def test_auth_wrong_token_rejected():
    reader, writer = make_stream_pair()
    token = bytes([0xDE] * AUTH_TOKEN_LEN)
    wrong = bytes([0xAD] * AUTH_TOKEN_LEN)

    await send_auth_token(writer, token)
    with pytest.raises(AuthError):
        await recv_auth_token(reader, wrong)


@pytest.mark.asyncio
async def test_auth_short_frame_rejected():
    reader, writer = make_stream_pair()
    expected = bytes([0xDE] * AUTH_TOKEN_LEN)

    # Send only 16 bytes (too short)
    await write_raw_frame(writer, bytes(16))
    with pytest.raises(AuthError):
        await recv_auth_token(reader, expected)
