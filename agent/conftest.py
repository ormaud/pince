"""Top-level conftest: ensure pince_proto's bare imports work."""
import sys
import os

# frontend_pb2 uses bare `import agent_pb2` so pince_proto/ must be on sys.path
_here = os.path.dirname(__file__)
_pince_proto_dir = os.path.join(_here, "pince_proto")
if _pince_proto_dir not in sys.path:
    sys.path.insert(0, _pince_proto_dir)
if _here not in sys.path:
    sys.path.insert(0, _here)
