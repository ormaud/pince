#!/usr/bin/env bash
cd /workspace
while :; do
  claude -p "$(cat denv/prompt.md)" --dangerously-skip-permissions --model "${MODEL:-sonnet}" || {
    echo "[agent-loop] claude exited $? — aborting"
    exit 1
  }
  if [ -f /tmp/.no-plan ]; then
    rm -f /tmp/.no-plan
    echo "[agent-loop] no plans available — sleeping 5 minutes"
    sleep 300
  fi
done
