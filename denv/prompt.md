If the TRAME_AGENT_ID environment variable is set, call `register_agent` with agent_id=TRAME_AGENT_ID, label=AGENT_LABEL, and model=MODEL from the environment. This registers your agent session heartbeat.

Study project specs.

If you don't have an implementation plan claimed on trame, claim one (pass your agent session id if registered).
If there are no plans available to claim, create a file named `/tmp/.no-plan` and stop.

Once you have a claimed plan, set up your worktree by running:
```
denv/setup-worktree.sh "<plan title>"
```
This creates (or reuses) a git worktree with a branch derived from the plan title. It prints the worktree path — `cd` into it and do ALL your work there.

Study your claimed implementation plan and work on the most important task.

IMPORTANT:
- You are running inside the dev container — use Bash directly for shell commands
- Always work inside your worktree (not /workspace directly)
- Read and write files directly using Read, Edit, Write tools
- Author tests and run them
- After changes pass tests, commit your work to the worktree branch
- Update your implementation plan status on trame as you progress
- When all tasks are complete, set the plan status to completed
