#!/usr/bin/env bash
set -euo pipefail

# Usage: denv/agent.sh [--model MODEL] [--role agent|reviewer]
# Launches a headless Claude Code agent in a compose stack.

# --- Parse arguments ---
MODEL=""
ROLE="agent"
while [[ $# -gt 0 ]]; do
  case "$1" in
  --model)
    MODEL="$2"
    shift 2
    ;;
  --role)
    ROLE="$2"
    shift 2
    ;;
  *)
    echo "Unknown option: $1"
    exit 1
    ;;
  esac
done

# Validate role
case "$ROLE" in
  agent|reviewer) ;;
  *) echo "Invalid role: $ROLE (must be agent or reviewer)"; exit 1 ;;
esac

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT="$(basename "$PROJECT_DIR")"

# --- Load .env for API keys ---
ENV_FILE="$PROJECT_DIR/.env"
if [ -f "$ENV_FILE" ]; then
  set -a; source "$ENV_FILE"; set +a
fi

# Select API key based on role
if [ "$ROLE" = "reviewer" ]; then
  TRAME_API_KEY="${TRAME_API_KEY_REVIEWER:-}"
else
  TRAME_API_KEY="${TRAME_API_KEY_AGENT:-}"
fi

if [ -z "$TRAME_API_KEY" ]; then
  KEY_VAR="TRAME_API_KEY_$(echo "$ROLE" | tr '[:lower:]' '[:upper:]')"
  echo "Error: $KEY_VAR not set in .env"
  exit 1
fi

# --- Auto-increment instance ID ---
PREFIX="${PROJECT}-${ROLE}"
EXISTING=$(docker compose ls --format json 2>/dev/null |
  grep -oP "\"${PREFIX}-\K[0-9]+" |
  sort -n | tail -1 || true)
INSTANCE_ID=$((${EXISTING:-0} + 1))
INSTANCE_NAME="${PREFIX}-${INSTANCE_ID}"

echo "Starting $INSTANCE_NAME ..."

# --- Cleanup on exit ---
cleanup() {
  echo ""
  echo "Stopping $INSTANCE_NAME ..."
  docker compose -p "$INSTANCE_NAME" -f "$SCRIPT_DIR/docker-compose.worker.yml" down 2>/dev/null || true
}
trap cleanup EXIT

# --- Export environment for compose ---
export LOCAL_UID="$(id -u)"
export LOCAL_GID="$(id -g)"
export DENV_IMAGE="${DENV_IMAGE:-${PROJECT}-denv}"
export MODEL
export ROLE
export TRAME_API_KEY

# --- Start compose stack ---
docker compose -p "$INSTANCE_NAME" -f "$SCRIPT_DIR/docker-compose.worker.yml" up -d

# --- Follow logs (foreground, Ctrl+C to stop) ---
docker compose -p "$INSTANCE_NAME" -f "$SCRIPT_DIR/docker-compose.worker.yml" logs -f devenv
