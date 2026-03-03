"""Framing layer for the pince protocol.

Frame format: [4 bytes: u32 BE length][N bytes: protobuf-encoded message]
Max message size: 16 MiB.

Same framing as the Rust implementation — binary compatible.
"""
from __future__ import annotations

import asyncio
import struct
from typing import Type, TypeVar

from google.protobuf.message import Message

MAX_MESSAGE_SIZE = 16 * 1024 * 1024  # 16 MiB
_HEADER_FMT = ">I"  # big-endian uint32
_HEADER_SIZE = struct.calcsize(_HEADER_FMT)


class FramingError(Exception):
    """Raised when a framing violation is detected."""


class MessageTooLargeError(FramingError):
    """Raised when the message length exceeds MAX_MESSAGE_SIZE."""

    def __init__(self, size: int) -> None:
        super().__init__(f"message too large: {size} bytes (max {MAX_MESSAGE_SIZE})")
        self.size = size


T = TypeVar("T", bound=Message)


async def read_message(
    reader: asyncio.StreamReader,
    msg_type: Type[T],
) -> T:
    """Read a length-prefixed protobuf message from a StreamReader."""
    header = await reader.readexactly(_HEADER_SIZE)
    (length,) = struct.unpack(_HEADER_FMT, header)

    if length > MAX_MESSAGE_SIZE:
        raise MessageTooLargeError(length)

    data = await reader.readexactly(length)
    msg = msg_type()
    msg.ParseFromString(data)
    return msg


async def write_message(
    writer: asyncio.StreamWriter,
    msg: Message,
) -> None:
    """Write a length-prefixed protobuf message to a StreamWriter."""
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
