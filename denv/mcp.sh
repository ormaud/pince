#!/usr/bin/env bash
# MCP launcher — starts the compose stack and execs into the MCP server.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT="$(basename "$PROJECT_DIR")"
COMPOSE_PROJECT="${PROJECT}-coord"
COMPOSE_FILE="$SCRIPT_DIR/docker-compose.coord.yml"

export LOCAL_UID=$(id -u)
export LOCAL_GID=$(id -g)
export DENV_IMAGE="${DENV_IMAGE:-${PROJECT}-denv}"

# Ensure stack is running
docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" up -d --wait 2>/dev/null

# Exec MCP server — replaces this shell so stdio flows to Claude Code
# --user is needed because `docker compose exec` bypasses the entrypoint,
# which would otherwise drop privileges via gosu.
exec docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" exec -T --user "$LOCAL_UID:$LOCAL_GID" devenv mcp-server
