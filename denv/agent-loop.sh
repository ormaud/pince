#!/usr/bin/env bash
cd /workspace

# Select prompt based on role
PROMPT_FILE="denv/prompt.md"
if [ "${ROLE:-agent}" = "reviewer" ]; then
  PROMPT_FILE="denv/reviewer-prompt.md"
fi

echo "[agent-loop] role=${ROLE:-agent} model=${MODEL:-sonnet} prompt=$PROMPT_FILE"

# Configure trame MCP with API key auth
if [ -n "${TRAME_API_KEY:-}" ]; then
  mkdir -p "$HOME/.claude"
  cat > "$HOME/.claude/settings.json" <<EOF
{
  "mcpServers": {
    "trame": {
      "type": "url",
      "url": "https://trame.sh/mcp",
      "headers": {
        "Authorization": "Bearer $TRAME_API_KEY"
      }
    }
  }
}
EOF
fi

while :; do
  claude -p "$(cat "$PROMPT_FILE")" --dangerously-skip-permissions --model "${MODEL:-sonnet}" || {
    echo "[agent-loop] claude exited $? — aborting"
    exit 1
  }
  if [ -f /tmp/.no-plan ]; then
    rm -f /tmp/.no-plan
    echo "[agent-loop] no plans available — sleeping 5 minutes"
    sleep 300
  fi
done
