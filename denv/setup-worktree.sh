#!/usr/bin/env bash
set -euo pipefail

# Usage: denv/setup-worktree.sh <branch-name>
# Creates or reuses a git worktree for a branch.
# Prints the worktree path on success.

BRANCH="${1:?Usage: setup-worktree.sh <branch-name>}"

# Extract directory name from branch (e.g. plan/a1b2c3d4-add-auth → a1b2c3d4-add-auth)
DIR_NAME="${BRANCH##*/}"
WORKTREE="/workspace/worktrees/${DIR_NAME}"

# If worktree already exists, reuse it
if [ -d "$WORKTREE" ]; then
  echo "$WORKTREE"
  exit 0
fi

mkdir -p /workspace/worktrees

# Fetch latest refs
git -C /workspace fetch origin

# If branch exists locally, create worktree from it
if git -C /workspace show-ref --verify --quiet "refs/heads/${BRANCH}" 2>/dev/null; then
  git -C /workspace worktree add "$WORKTREE" "$BRANCH"
# If branch exists on remote, track it
elif git -C /workspace show-ref --verify --quiet "refs/remotes/origin/${BRANCH}" 2>/dev/null; then
  git -C /workspace worktree add "$WORKTREE" "$BRANCH"
else
  git -C /workspace worktree add "$WORKTREE" -b "$BRANCH" origin/main
fi

echo "$WORKTREE"
