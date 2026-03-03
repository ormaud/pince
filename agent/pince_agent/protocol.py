"""Protocol layer for pince agent: framing + auth token exchange.

Adapted from agent/codec.py and agent/auth.py.
"""
from __future__ import annotations

import asyncio
import hmac
import os
import socket
import struct
from typing import Type, TypeVar

from google.protobuf.message import Message

MAX_MESSAGE_SIZE = 16 * 1024 * 1024  # 16 MiB
_HEADER_FMT = ">I"  # big-endian uint32
_HEADER_SIZE = struct.calcsize(_HEADER_FMT)

AUTH_TOKEN_ENV = "PINCE_AUTH_TOKEN"
SOCKET_FD_ENV = "PINCE_SOCKET_FD"
AUTH_TOKEN_LEN = 32  # bytes


class FramingError(Exception):
    """Raised when a framing violation is detected."""


class MessageTooLargeError(FramingError):
    def __init__(self, size: int) -> None:
        super().__init__(f"message too large: {size} bytes (max {MAX_MESSAGE_SIZE})")
        self.size = size


class AuthError(Exception):
    """Raised on authentication failure."""


T = TypeVar("T", bound=Message)


async def read_message(reader: asyncio.StreamReader, msg_type: Type[T]) -> T:
    """Read a length-prefixed protobuf message."""
    header = await reader.readexactly(_HEADER_SIZE)
    (length,) = struct.unpack(_HEADER_FMT, header)
    if length > MAX_MESSAGE_SIZE:
        raise MessageTooLargeError(length)
    data = await reader.readexactly(length)
    msg = msg_type()
    msg.ParseFromString(data)
    return msg


async def write_message(writer: asyncio.StreamWriter, msg: Message) -> None:
    """Write a length-prefixed protobuf message."""
    data = msg.SerializeToString()
    length = len(data)
    if length > MAX_MESSAGE_SIZE:
        raise MessageTooLargeError(length)
    header = struct.pack(_HEADER_FMT, length)
    writer.write(header + data)
    await writer.drain()


async def read_raw_frame(reader: asyncio.StreamReader) -> bytes:
    """Read a raw length-prefixed frame (used for auth token exchange)."""
    header = await reader.readexactly(_HEADER_SIZE)
    (length,) = struct.unpack(_HEADER_FMT, header)
    if length > MAX_MESSAGE_SIZE:
        raise MessageTooLargeError(length)
    return await reader.readexactly(length)


async def write_raw_frame(writer: asyncio.StreamWriter, data: bytes) -> None:
    """Write a raw length-prefixed frame (used for auth token exchange)."""
    length = len(data)
    if length > MAX_MESSAGE_SIZE:
        raise MessageTooLargeError(length)
    header = struct.pack(_HEADER_FMT, length)
    writer.write(header + data)
    await writer.drain()


def load_token_from_env() -> bytes:
    """Load and parse the auth token from the environment variable."""
    hex_token = os.environ.get(AUTH_TOKEN_ENV)
    if not hex_token:
        raise AuthError(f"Missing environment variable: {AUTH_TOKEN_ENV}")
    return parse_auth_token(hex_token)


def parse_auth_token(hex_token: str) -> bytes:
    """Parse a hex-encoded auth token."""
    if len(hex_token) != AUTH_TOKEN_LEN * 2:
        raise AuthError(
            f"Invalid token length: expected {AUTH_TOKEN_LEN * 2} hex chars, "
            f"got {len(hex_token)}"
        )
    try:
        token = bytes.fromhex(hex_token)
    except ValueError as e:
        raise AuthError(f"Invalid hex token: {e}") from e
    return token


async def send_auth_token(writer: asyncio.StreamWriter, token: bytes) -> None:
    """Send the auth token as the first raw frame."""
    if len(token) != AUTH_TOKEN_LEN:
        raise AuthError(f"Token must be {AUTH_TOKEN_LEN} bytes")
    await write_raw_frame(writer, token)


async def recv_auth_token(reader: asyncio.StreamReader, expected: bytes) -> None:
    """Read and validate the auth token. Raises AuthError if invalid."""
    if len(expected) != AUTH_TOKEN_LEN:
        raise AuthError(f"Expected token must be {AUTH_TOKEN_LEN} bytes")
    try:
        frame = await read_raw_frame(reader)
    except Exception as e:
        raise AuthError(f"Failed to read auth token: {e}") from e
    if len(frame) != AUTH_TOKEN_LEN:
        raise AuthError("Invalid auth token length")
    if not hmac.compare_digest(frame, expected):
        raise AuthError("Auth token mismatch")


def open_socket_from_env() -> socket.socket:
    """Open the Unix socket from PINCE_SOCKET_FD environment variable."""
    fd_str = os.environ.get(SOCKET_FD_ENV)
    if not fd_str:
        raise OSError(f"Missing environment variable: {SOCKET_FD_ENV}")
    fd = int(fd_str)
    return socket.fromfd(fd, socket.AF_UNIX, socket.SOCK_STREAM)
