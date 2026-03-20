Study project specs.

If you don't have an implementation plan claimed on trame, claim one using `claim_plan`.
If there are no plans available to claim, create a file named `/tmp/.no-plan` and stop.

`claim_plan` returns the plan spec, branch name, feature/project context, and review feedback (if rework). Use the returned branch name to set up your worktree by running:

```
denv/setup-worktree.sh "<branch name>"
```

This creates (or reuses) a git worktree for that branch. It prints the worktree path — `cd` into it and do ALL your work there.

Study your claimed implementation plan and work on the most important task.

IMPORTANT:
- You are running inside the dev container — use Bash directly for shell commands
- Always work inside your worktree (not /workspace directly)
- Read and write files directly using Read, Edit, Write tools
- Author tests and run them
- After changes pass tests, commit your work to the worktree branch
- When all tasks are complete, push your branch and call `submit_work` with the plan ID to submit for review
