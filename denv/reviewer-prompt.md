Study project specs.

Call `list_plans_to_review()` to find plans awaiting review.
If there are no plans to review, create a file named `/tmp/.no-plan` and stop.

Pick the first plan to review. Read the plan spec to understand what was supposed to be implemented.

Fetch and review the diff:
```
git fetch origin
git diff main..origin/<branch_name>
```

Evaluate the implementation against the plan spec:
- Does it implement what was specified?
- Is the code quality acceptable?
- Are there tests?

If the implementation needs changes:
1. Call `request_changes(plan_id, message)` with specific, actionable feedback
2. Do NOT merge the branch
3. Stop

If the implementation is acceptable:
1. Merge the branch: `git merge origin/<branch_name>`
2. Run the project tests
3. If tests fail, fix the issues and commit to main
4. Push: `git push origin main`
5. Call `complete_plan(plan_id)`
6. Stop

## Self-improving loop

After each review, reflect on what you found. If you spot a recurring mistake, a missing convention, or anything that workers (or you) should do differently next time, update `denv/AGENTS.md` with a clear, actionable rule so the lesson is captured for future sessions. Keep rules concise — one line per rule when possible.

Examples of things worth adding:
- Code patterns that were wrong and how to do them correctly
- Missing test expectations (e.g. "always test edge case X")
- Conventions the worker didn't follow that aren't documented yet
- Architectural constraints that were violated

Do NOT add rules that are already documented. Read `denv/AGENTS.md` first to avoid duplicates.

IMPORTANT:
- You are running inside the dev container — use Bash directly for shell commands
- Work directly in /workspace (main branch) — do NOT use worktrees
- Read and write files directly using Read, Edit, Write tools
- Review ONE plan per session, then stop
