"""One-time auth token protocol for pince agents.

The supervisor generates a 32-byte random token per agent at spawn time.
The token is passed via PINCE_AUTH_TOKEN env variable (hex-encoded).
The agent sends the token as its first raw frame before any protobuf messages.
"""
from __future__ import annotations

import hmac
import os
import asyncio
from codec import read_raw_frame, write_raw_frame

AUTH_TOKEN_ENV = "PINCE_AUTH_TOKEN"
AUTH_TOKEN_LEN = 32  # bytes


class AuthError(Exception):
    """Raised on authentication failure."""


def load_token_from_env() -> bytes:
    """Load and parse the auth token from the environment variable."""
    hex_token = os.environ.get(AUTH_TOKEN_ENV)
    if not hex_token:
        raise AuthError(f"Missing environment variable: {AUTH_TOKEN_ENV}")
    return parse_token(hex_token)


def parse_token(hex_token: str) -> bytes:
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
    """Read and validate the auth token from the first raw frame.

    Uses constant-time comparison to prevent timing attacks.
    Raises AuthError if the token is invalid.
    """
    if len(expected) != AUTH_TOKEN_LEN:
        raise AuthError(f"Expected token must be {AUTH_TOKEN_LEN} bytes")

    try:
        frame = await read_raw_frame(reader)
    except Exception as e:
        raise AuthError(f"Failed to read auth token: {e}") from e

    if len(frame) != AUTH_TOKEN_LEN:
        raise AuthError("Invalid auth token length")

    # Constant-time comparison
    if not hmac.compare_digest(frame, expected):
        raise AuthError("Auth token mismatch")
