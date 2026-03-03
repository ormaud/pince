#!/usr/bin/env bash
# Wrapped in { } so bash reads the entire script before executing,
# allowing the self-update below to safely overwrite this file.
{
set -euo pipefail

REPO="trame-sh/trame-tools"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VERSION_FILE="$SCRIPT_DIR/.trame-tools-version"

get_latest_version() {
  curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | grep '"tag_name"' | cut -d'"' -f4
}

# --check / -c: compare local version against latest release
if [ "${1:-}" = "--check" ] || [ "${1:-}" = "-c" ]; then
  LATEST=$(get_latest_version)
  if [ -f "$VERSION_FILE" ]; then
    CURRENT=$(cat "$VERSION_FILE")
    if [ "$CURRENT" = "$LATEST" ]; then
      echo "Up to date ($CURRENT)"
    else
      echo "Update available: $CURRENT → $LATEST"
      echo "Run \`denv/update.sh\` to update, then rebuild with \`denv/build-denv.sh\`."
    fi
  else
    echo "Version unknown (no .trame-tools-version file). Run \`denv/update.sh\` to install latest ($LATEST)."
  fi
  exit 0
fi

# Default: update to specified or latest version
VERSION="${1:-latest}"
if [ "$VERSION" = "latest" ]; then
  VERSION=$(get_latest_version)
fi

BASE="https://raw.githubusercontent.com/$REPO/$VERSION/tools"

# Self-update: if the new version differs, re-exec it so new downloads are picked up
curl -fsSL "$BASE/update.sh" -o "$SCRIPT_DIR/update.sh.new"
if ! cmp -s "$SCRIPT_DIR/update.sh" "$SCRIPT_DIR/update.sh.new"; then
  mv "$SCRIPT_DIR/update.sh.new" "$SCRIPT_DIR/update.sh"
  chmod +x "$SCRIPT_DIR/update.sh"
  exec "$SCRIPT_DIR/update.sh" "$VERSION"
else
  rm -f "$SCRIPT_DIR/update.sh.new"

  curl -fsSL "$BASE/mcp.sh" -o "$SCRIPT_DIR/mcp.sh"
  chmod +x "$SCRIPT_DIR/mcp.sh"

  curl -fsSL "$BASE/agent.sh" -o "$SCRIPT_DIR/agent.sh"
  chmod +x "$SCRIPT_DIR/agent.sh"

  curl -fsSL "$BASE/agent-loop.sh" -o "$SCRIPT_DIR/agent-loop.sh"
  chmod +x "$SCRIPT_DIR/agent-loop.sh"

  curl -fsSL "$BASE/setup-worktree.sh" -o "$SCRIPT_DIR/setup-worktree.sh"
  chmod +x "$SCRIPT_DIR/setup-worktree.sh"

  curl -fsSL "$BASE/build-denv.sh" -o "$SCRIPT_DIR/build-denv.sh"
  chmod +x "$SCRIPT_DIR/build-denv.sh"

  curl -fsSL "$BASE/AGENTS.md" -o "$SCRIPT_DIR/AGENTS.md"

  # prompt.md is user-customizable — only download if missing
  if [ ! -f "$SCRIPT_DIR/prompt.md" ]; then
    curl -fsSL "$BASE/prompt.md" -o "$SCRIPT_DIR/prompt.md"
  fi

  PROJECT_DIR="$SCRIPT_DIR/.."
  mkdir -p "$PROJECT_DIR/.claude/agents"
  curl -fsSL "$BASE/teammate.md" -o "$PROJECT_DIR/.claude/agents/teammate.md"

  echo "$VERSION" >"$VERSION_FILE"

  echo "Updated to $REPO@$VERSION"
fi
}
