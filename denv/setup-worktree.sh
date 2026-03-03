#!/usr/bin/env bash
set -euo pipefail

# Usage: denv/setup-worktree.sh <plan-title>
# Creates or reuses a git worktree for a plan.
# The branch name is derived deterministically from the plan title.
# Prints the worktree path on success.

PLAN_TITLE="${1:?Usage: setup-worktree.sh <plan-title>}"

# Slugify: lowercase, replace non-alphanum with hyphens, collapse, trim
SLUG=$(echo "$PLAN_TITLE" | tr '[:upper:]' '[:lower:]' | sed 's/[^a-z0-9]/-/g; s/--*/-/g; s/^-//; s/-$//')
BRANCH="plan/${SLUG}"
WORKTREE="/workspace/worktrees/${SLUG}"

# If worktree already exists, reuse it
if [ -d "$WORKTREE" ]; then
  echo "$WORKTREE"
  exit 0
fi

mkdir -p /workspace/worktrees

# If branch exists, create worktree from it (resuming work)
if git -C /workspace show-ref --verify --quiet "refs/heads/${BRANCH}" 2>/dev/null; then
  git -C /workspace worktree add "$WORKTREE" "$BRANCH"
else
  git -C /workspace worktree add "$WORKTREE" -b "$BRANCH" origin/main
fi

echo "$WORKTREE"
