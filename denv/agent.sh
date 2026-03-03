#!/usr/bin/env bash
set -euo pipefail

# Usage: denv/agent.sh [--model MODEL]
# Launches a headless Claude Code agent in an isolated worktree + compose stack.

# --- Parse arguments ---
MODEL=""
while [[ $# -gt 0 ]]; do
  case "$1" in
  --model)
    MODEL="$2"
    shift 2
    ;;
  *)
    echo "Unknown option: $1"
    exit 1
    ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT="$(basename "$PROJECT_DIR")"

# --- Auto-increment worker ID ---
EXISTING=$(docker compose ls --format json 2>/dev/null |
  grep -oP "\"${PROJECT}-worker-\K[0-9]+" |
  sort -n | tail -1 || true)
WORKER_ID=$((${EXISTING:-0} + 1))
WORKER_NAME="${PROJECT}-worker-${WORKER_ID}"

echo "Starting $WORKER_NAME ..."

# --- Cleanup on exit ---
cleanup() {
  echo ""
  echo "Stopping $WORKER_NAME ..."
  docker compose -p "$WORKER_NAME" -f "$SCRIPT_DIR/docker-compose.worker.yml" down 2>/dev/null || true
}
trap cleanup EXIT

# --- Export environment for compose ---
export LOCAL_UID="$(id -u)"
export LOCAL_GID="$(id -g)"
export DENV_IMAGE="${DENV_IMAGE:-${PROJECT}-denv}"
export MODEL
export AGENT_LABEL="worker-${WORKER_ID}"
export TRAME_AGENT_ID=$(uuidgen 2>/dev/null || cat /proc/sys/kernel/random/uuid)

# --- Start compose stack ---
docker compose -p "$WORKER_NAME" -f "$SCRIPT_DIR/docker-compose.worker.yml" up -d

# --- Follow logs (foreground, Ctrl+C to stop) ---
docker compose -p "$WORKER_NAME" -f "$SCRIPT_DIR/docker-compose.worker.yml" logs -f devenv
