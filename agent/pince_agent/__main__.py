"""Entry point for the pince sub-agent: python -m pince_agent."""
from __future__ import annotations

import asyncio
import logging
import os
import sys

# Ensure the parent agent/ directory (containing pince_proto) is on the path
import pathlib as _pathlib
_agent_dir = _pathlib.Path(__file__).parent.parent
sys.path.insert(0, str(_agent_dir))
# frontend_pb2 uses bare `import agent_pb2`, so pince_proto dir must also be on path
sys.path.insert(0, str(_agent_dir / "pince_proto"))

from pince_agent.agent import agent_loop
from pince_agent.protocol import (
    load_token_from_env,
    open_socket_from_env,
    read_message,
    send_auth_token,
    write_message,
)

from pince_proto import AgentMessage, Ready, SupervisorMessage  # noqa: E402

logging.basicConfig(
    level=os.environ.get("PINCE_LOG_LEVEL", "INFO"),
    format="%(asctime)s %(name)s %(levelname)s %(message)s",
    stream=sys.stderr,
)
logger = logging.getLogger("pince_agent")


async def main() -> None:
    logger.info("pince_agent starting")

    # 1. Load auth token from env
    auth_token = load_token_from_env()

    # 2. Open socket from inherited file descriptor
    sock = open_socket_from_env()
    sock.setblocking(False)

    # 3. Wrap in asyncio streams
    loop = asyncio.get_event_loop()
    reader, writer = await asyncio.open_unix_connection(sock=sock)

    try:
        # 4. Send auth token
        await send_auth_token(writer, auth_token)
        logger.debug("auth token sent")

        # 5. Receive Init message
        init_msg = await read_message(reader, SupervisorMessage)
        if not init_msg.HasField("init"):
            raise RuntimeError(f"Expected Init message, got: {init_msg}")
        config = init_msg.init.config
        logger.info("received Init: model=%s agent_id=%s", config.model, config.agent_id)

        # 6. Send Ready
        ready_msg = AgentMessage()
        ready_msg.ready.CopyFrom(Ready())
        await write_message(writer, ready_msg)
        logger.info("ready")

        # 7. Enter main loop
        await agent_loop(reader, writer, config)

    finally:
        writer.close()
        try:
            await writer.wait_closed()
        except Exception:
            pass

    logger.info("pince_agent exiting")


def main_sync() -> None:
    asyncio.run(main())


if __name__ == "__main__":
    main_sync()
