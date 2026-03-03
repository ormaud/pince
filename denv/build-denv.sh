#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT="$(basename "$(cd "$SCRIPT_DIR/.." && pwd)")"
IMAGE="${DENV_IMAGE:-${PROJECT}-denv}"

echo "Building $IMAGE (no cache) ..."
docker build "$SCRIPT_DIR" --no-cache --pull -t "$IMAGE"
echo "Done: $IMAGE"
