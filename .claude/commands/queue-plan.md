---
description: Queue a plan file as a Ready task in the task orchestrator kanban board
allowed-tools: Bash, Glob, Read
---

Queue a plan as a task in the task orchestrator.

## Instructions

1. **Find the plan file:**
   - If an argument was provided ("$ARGUMENTS"), use that as the plan file path
   - Otherwise, use Glob to find the most recently modified `.md` file in `docs/superpowers/plans/` or `docs/plans/`
   - If no plan file is found, ask the user for the path

2. **Run the CLI to create the task:**
   ```
   task-orchestrator create --from-plan <absolute-path> --repo-path <current-working-directory>
   ```

3. **Report the result to the user.** Show the task ID and title from the CLI output.
