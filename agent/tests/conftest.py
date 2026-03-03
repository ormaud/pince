"""Shared test fixtures and helpers."""
import asyncio
import sys
import os

# Ensure agent/ and pince_proto are importable
agent_dir = os.path.dirname(os.path.dirname(__file__))
sys.path.insert(0, agent_dir)
# frontend_pb2 uses bare `import agent_pb2` so we also need pince_proto on sys.path
pince_proto_dir = os.path.join(agent_dir, "pince_proto")
sys.path.insert(0, pince_proto_dir)
