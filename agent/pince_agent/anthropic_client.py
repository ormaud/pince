"""Anthropic SDK wrapper for the pince agent."""
from __future__ import annotations

import json
from typing import Any, AsyncIterator

import anthropic


class AnthropicClient:
    """Thin wrapper around the Anthropic async streaming API."""

    def __init__(self, model: str, system_prompt: str, max_tokens: int = 4096) -> None:
        self.client = anthropic.AsyncAnthropic()  # reads ANTHROPIC_API_KEY from env
        self.model = model
        self.system_prompt = system_prompt
        self.max_tokens = max_tokens

    async def stream_response(
        self,
        messages: list[dict[str, Any]],
        tools: list[dict[str, Any]],
    ) -> AsyncIterator[anthropic.types.MessageStreamEvent]:
        """Stream a response from the Anthropic API.

        Yields raw stream events. Callers process text deltas and tool_use blocks.
        """
        kwargs: dict[str, Any] = {
            "model": self.model,
            "system": self.system_prompt,
            "messages": messages,
            "max_tokens": self.max_tokens,
        }
        if tools:
            kwargs["tools"] = tools

        async with self.client.messages.stream(**kwargs) as stream:
            async for event in stream:
                yield event


def convert_tool_schemas(tool_schemas: list[Any]) -> list[dict[str, Any]]:
    """Convert protobuf ToolSchema objects to Anthropic tool format."""
    result = []
    for schema in tool_schemas:
        input_schema = json.loads(schema.input_schema_json)
        result.append(
            {
                "name": schema.name,
                "description": schema.description,
                "input_schema": input_schema,
            }
        )
    return result
